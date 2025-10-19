use super::lights::{resolve_light_transform, safe_normalize};
use super::rendering::{apply_billboard_transform, CameraVectors};
use crate::asset::{Assets, Handle, Mesh};
use crate::renderer::batch::{CullMode, InstanceSource, RenderObject, RenderPass};
use crate::renderer::primitives::{cone_side_mesh, cylinder_mesh, quad_mesh, sphere_mesh};
use crate::renderer::texture::Texture;
use crate::renderer::{Material, Renderer};
use crate::scene::components::{
    Billboard, BillboardOrientation, DepthState, DirectionalLight, PointLight, SelectedInEditor,
    SpotLight, TransformComponent, WorldTransform,
};
use crate::scene::transform::Transform;
use glam::{Quat, Vec3};
use hecs::World;

#[derive(Clone, Copy)]
pub(crate) struct GizmoResources {
    pub quad: Handle<Mesh>,
    pub sphere: Handle<Mesh>,
    pub cone_shell: Handle<Mesh>,
    pub cylinder: Handle<Mesh>,
    pub point_icon: Handle<Texture>,
    pub spot_icon: Handle<Texture>,
    pub directional_icon: Handle<Texture>,
}

const POINT_SPRITE_COLOR: [u8; 4] = [255, 226, 120, 255];
const SPOT_SPRITE_COLOR: [u8; 4] = [120, 220, 255, 255];
const DIRECTIONAL_SPRITE_COLOR: [u8; 4] = [140, 180, 255, 255];
const LIGHT_GEOMETRY_ALPHA: u8 = 128; // ~50% transparency

const POINT_VOLUME_COLOR: [u8; 4] = [255, 226, 120, LIGHT_GEOMETRY_ALPHA];
const SPOT_VOLUME_COLOR: [u8; 4] = [120, 220, 255, LIGHT_GEOMETRY_ALPHA];
const SPOT_INNER_OUTLINE_COLOR: [u8; 4] = [120, 220, 255, LIGHT_GEOMETRY_ALPHA];
const SPOT_OUTER_OUTLINE_COLOR: [u8; 4] = [120, 220, 255, LIGHT_GEOMETRY_ALPHA];
const DIRECTIONAL_SHAFT_COLOR: [u8; 4] = [140, 180, 255, LIGHT_GEOMETRY_ALPHA];
const LIGHT_GEOMETRY_DEPTH: DepthState = DepthState::new(false, false);

const DIRECTIONAL_SHAFT_LENGTH: f32 = 2.25;
const DIRECTIONAL_SHAFT_RADIUS: f32 = 0.12;

const ICON_SCREEN_FRACTION: f32 = 0.055;
const ICON_MIN_DISTANCE: f32 = 0.1;
const ICON_ORTHO_WORLD_SIZE: f32 = 0.65;

const POINT_ICON_BYTES: &[u8] = include_bytes!("../../bin/assets/point_light.png");
const SPOT_ICON_BYTES: &[u8] = include_bytes!("../../bin/assets/spot_light.png");
const DIRECTIONAL_ICON_BYTES: &[u8] = include_bytes!("../../bin/assets/directional_light.png");

pub(crate) fn create_resources(renderer: &mut Renderer, assets: &mut Assets) -> GizmoResources {
    let (quad_vertices, quad_indices) = quad_mesh();
    let quad_mesh = renderer.create_mesh(&quad_vertices, &quad_indices);
    let quad = assets.meshes.insert(quad_mesh);

    let (sphere_vertices, sphere_indices) = sphere_mesh(12, 8);
    let sphere_mesh = renderer.create_mesh(&sphere_vertices, &sphere_indices);
    let sphere = assets.meshes.insert(sphere_mesh);

    let (cone_shell_vertices, cone_shell_indices) = cone_side_mesh(16);
    let cone_shell_mesh = renderer.create_mesh(&cone_shell_vertices, &cone_shell_indices);
    let cone_shell = assets.meshes.insert(cone_shell_mesh);

    let (cylinder_vertices, cylinder_indices) = cylinder_mesh(16);
    let cylinder_mesh = renderer.create_mesh(&cylinder_vertices, &cylinder_indices);
    let cylinder = assets.meshes.insert(cylinder_mesh);

    let point_icon = load_icon_texture(renderer, assets, POINT_ICON_BYTES, "PointLightIcon");
    let spot_icon = load_icon_texture(renderer, assets, SPOT_ICON_BYTES, "SpotLightIcon");
    let directional_icon = load_icon_texture(
        renderer,
        assets,
        DIRECTIONAL_ICON_BYTES,
        "DirectionalLightIcon",
    );
    renderer.update_texture_bind_group(assets);

    GizmoResources {
        quad,
        sphere,
        cone_shell,
        cylinder,
        point_icon,
        spot_icon,
        directional_icon,
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
    for (_entity, (light, world_transform, local_transform, selected)) in world
        .query::<(
            &PointLight,
            Option<&WorldTransform>,
            Option<&TransformComponent>,
            Option<&SelectedInEditor>,
        )>()
        .iter()
    {
        let mut transform = resolve_light_transform(world_transform, local_transform);
        transform = root_transform.mul_transform(&transform);

        let billboard = Billboard::new(BillboardOrientation::FaceCamera);
        let icon_scale = icon_world_scale(camera, transform.translation);
        let icon_transform = apply_billboard_transform(
            Transform::from_trs(
                transform.translation,
                Quat::IDENTITY,
                Vec3::splat(icon_scale),
            ),
            billboard,
            camera.position,
            camera.target,
            camera.up,
        );

        output.push(RenderObject {
            mesh: resources.quad,
            material: sprite_material(POINT_SPRITE_COLOR, resources.point_icon.index() as u32),
            transform: icon_transform,
            depth_state: DepthState::new(false, false),
            force_overlay: false,
            render_pass: Some(RenderPass::GizmoSolid),
            instance_source: InstanceSource::Cpu,
            gpu_index: None,
            cull_mode: CullMode::Back,
        });

        if selected.is_some() {
            let radius = light.range.max(0.01);
            output.push(RenderObject {
                mesh: resources.sphere,
                material: volume_material(POINT_VOLUME_COLOR),
                transform: Transform::from_trs(
                    transform.translation,
                    Quat::IDENTITY,
                    Vec3::splat(radius),
                ),
                depth_state: LIGHT_GEOMETRY_DEPTH,
                force_overlay: false,
                render_pass: Some(RenderPass::Gizmo),
                instance_source: InstanceSource::Cpu,
                gpu_index: None,
                cull_mode: CullMode::None,
            });
        }
    }
}

fn build_spot_light_gizmos(
    world: &World,
    camera: CameraVectors,
    root_transform: Transform,
    resources: GizmoResources,
    output: &mut Vec<RenderObject>,
) {
    for (_entity, (light, world_transform, local_transform, selected)) in world
        .query::<(
            &SpotLight,
            Option<&WorldTransform>,
            Option<&TransformComponent>,
            Option<&SelectedInEditor>,
        )>()
        .iter()
    {
        let mut transform = resolve_light_transform(world_transform, local_transform);
        transform = root_transform.mul_transform(&transform);

        let billboard = Billboard::new(BillboardOrientation::FaceCamera);
        let icon_scale = icon_world_scale(camera, transform.translation);
        let icon_transform = apply_billboard_transform(
            Transform::from_trs(
                transform.translation,
                Quat::IDENTITY,
                Vec3::splat(icon_scale),
            ),
            billboard,
            camera.position,
            camera.target,
            camera.up,
        );

        output.push(RenderObject {
            mesh: resources.quad,
            material: sprite_material(SPOT_SPRITE_COLOR, resources.spot_icon.index() as u32),
            transform: icon_transform,
            depth_state: DepthState::new(false, false),
            force_overlay: false,
            render_pass: Some(RenderPass::GizmoSolid),
            instance_source: InstanceSource::Cpu,
            gpu_index: None,
            cull_mode: CullMode::Back,
        });

        if selected.is_some() {
            let range = light.range.max(0.01);
            let outer_angle = light.outer_angle.max(0.01);
            let inner_angle = light.inner_angle.clamp(0.0, outer_angle);
            let outer_radius = (outer_angle.tan() * range).max(0.05);
            let inner_radius = (inner_angle.tan() * range).max(0.02);
            let rotation = transform.rotation;

            // Outer cone volume
            output.push(RenderObject {
                mesh: resources.cone_shell,
                material: volume_material(SPOT_VOLUME_COLOR),
                transform: Transform::from_trs(
                    transform.translation,
                    rotation,
                    Vec3::new(outer_radius, outer_radius, range),
                ),
                depth_state: LIGHT_GEOMETRY_DEPTH,
                force_overlay: false,
                render_pass: Some(RenderPass::Gizmo),
                instance_source: InstanceSource::Cpu,
                gpu_index: None,
                cull_mode: CullMode::None,
            });

            // Inner shell: slightly inset to convey falloff thickness.
            let shell_scale = (inner_radius / outer_radius).clamp(0.0, 0.999).max(0.01);
            output.push(RenderObject {
                mesh: resources.cone_shell,
                material: volume_material([
                    SPOT_VOLUME_COLOR[0],
                    SPOT_VOLUME_COLOR[1],
                    SPOT_VOLUME_COLOR[2],
                    60,
                ]),
                transform: Transform::from_trs(
                    transform.translation,
                    rotation,
                    Vec3::new(
                        outer_radius * shell_scale,
                        outer_radius * shell_scale,
                        range,
                    ),
                ),
                depth_state: LIGHT_GEOMETRY_DEPTH,
                force_overlay: false,
                render_pass: Some(RenderPass::Gizmo),
                instance_source: InstanceSource::Cpu,
                gpu_index: None,
                cull_mode: CullMode::None,
            });

            // Outer outline to highlight total spread.
            output.push(RenderObject {
                mesh: resources.cone_shell,
                material: outline_material(SPOT_OUTER_OUTLINE_COLOR),
                transform: Transform::from_trs(
                    transform.translation,
                    rotation,
                    Vec3::new(outer_radius, outer_radius, range),
                ),
                depth_state: LIGHT_GEOMETRY_DEPTH,
                force_overlay: false,
                render_pass: Some(RenderPass::Gizmo),
                instance_source: InstanceSource::Cpu,
                gpu_index: None,
                cull_mode: CullMode::None,
            });

            // Inner outline clarifies the core beam threshold.
            output.push(RenderObject {
                mesh: resources.cone_shell,
                material: outline_material(SPOT_INNER_OUTLINE_COLOR),
                transform: Transform::from_trs(
                    transform.translation,
                    rotation,
                    Vec3::new(inner_radius, inner_radius, range),
                ),
                depth_state: LIGHT_GEOMETRY_DEPTH,
                force_overlay: false,
                render_pass: Some(RenderPass::Gizmo),
                instance_source: InstanceSource::Cpu,
                gpu_index: None,
                cull_mode: CullMode::None,
            });
        }
    }
}

fn build_directional_light_gizmos(
    world: &World,
    camera: CameraVectors,
    root_transform: Transform,
    resources: GizmoResources,
    output: &mut Vec<RenderObject>,
) {
    for (_entity, (_light, world_transform, local_transform, selected)) in world
        .query::<(
            &DirectionalLight,
            Option<&WorldTransform>,
            Option<&TransformComponent>,
            Option<&SelectedInEditor>,
        )>()
        .iter()
    {
        let mut transform = resolve_light_transform(world_transform, local_transform);
        transform = root_transform.mul_transform(&transform);
        let position = transform.translation;
        let direction = safe_normalize(transform.rotation * Vec3::NEG_Z, Vec3::NEG_Z);

        let billboard = Billboard::new(BillboardOrientation::FaceCamera);
        let icon_scale = icon_world_scale(camera, position);
        let icon_transform = apply_billboard_transform(
            Transform::from_trs(position, Quat::IDENTITY, Vec3::splat(icon_scale)),
            billboard,
            camera.position,
            camera.target,
            camera.up,
        );

        output.push(RenderObject {
            mesh: resources.quad,
            material: sprite_material(
                DIRECTIONAL_SPRITE_COLOR,
                resources.directional_icon.index() as u32,
            ),
            transform: icon_transform,
            depth_state: DepthState::new(false, false),
            force_overlay: false,
            render_pass: Some(RenderPass::GizmoSolid),
            instance_source: InstanceSource::Cpu,
            gpu_index: None,
            cull_mode: CullMode::Back,
        });

        if selected.is_some() {
            let shaft_rotation = align_vector(Vec3::Z, direction);
            let shaft_translation = position + direction * (DIRECTIONAL_SHAFT_LENGTH * 0.5);
            output.push(RenderObject {
                mesh: resources.cylinder,
                material: volume_material(DIRECTIONAL_SHAFT_COLOR),
                transform: Transform::from_trs(
                    shaft_translation,
                    shaft_rotation,
                    Vec3::new(
                        DIRECTIONAL_SHAFT_RADIUS,
                        DIRECTIONAL_SHAFT_RADIUS,
                        DIRECTIONAL_SHAFT_LENGTH,
                    ),
                ),
                depth_state: LIGHT_GEOMETRY_DEPTH,
                force_overlay: false,
                render_pass: Some(RenderPass::Gizmo),
                instance_source: InstanceSource::Cpu,
                gpu_index: None,
                cull_mode: CullMode::None,
            });
        }
    }
}

fn sprite_material(color: [u8; 4], texture_index: u32) -> Material {
    Material::new(color)
        .with_unlit()
        .with_alpha()
        .with_base_color_texture(texture_index)
}

fn volume_material(color: [u8; 4]) -> Material {
    let mut material = Material::new(color).with_unlit().with_alpha();
    material.base_color = color;
    material
}

fn outline_material(color: [u8; 4]) -> Material {
    Material::new(color).with_unlit().with_alpha()
}

fn align_vector(from: Vec3, direction: Vec3) -> Quat {
    if direction.length_squared() <= 0.0 {
        Quat::IDENTITY
    } else {
        Quat::from_rotation_arc(from, direction.normalize())
    }
}

fn icon_world_scale(camera: CameraVectors, position: Vec3) -> f32 {
    let distance = (camera.position - position).length().max(ICON_MIN_DISTANCE);
    let half_fov = (camera.fov_y * 0.5).max(1e-4);
    if half_fov <= 1e-3 {
        ICON_ORTHO_WORLD_SIZE
    } else {
        let vertical_extent = half_fov.tan();
        2.0 * distance * vertical_extent * ICON_SCREEN_FRACTION
    }
}

fn load_icon_texture(
    renderer: &Renderer,
    assets: &mut Assets,
    bytes: &[u8],
    label: &str,
) -> Handle<Texture> {
    let image = image::load_from_memory(bytes)
        .unwrap_or_else(|err| panic!("Failed to decode gizmo icon {label}: {err}"));
    let rgba = image.to_rgba8();
    let (width, height) = rgba.dimensions();
    let texture = Texture::from_bytes(
        renderer.get_device(),
        renderer.get_queue(),
        rgba.as_raw(),
        width,
        height,
        Some(label),
    );
    assets.textures.insert(texture)
}
