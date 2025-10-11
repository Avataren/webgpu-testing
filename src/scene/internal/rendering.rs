use super::lights::safe_normalize;
use crate::asset::{Handle, Mesh};
use crate::renderer::{batch::InstanceSource, Material, RenderObject, Renderer};
use crate::scene::components::{
    Billboard, BillboardOrientation, BillboardSpace, DepthState, GpuParticleInstance,
    MaterialComponent, MeshComponent, Name, TransformComponent, Visible, WorldTransform,
};
use crate::scene::transform::Transform;
use glam::{Mat3, Quat, Vec3};
use hecs::World;
use rayon::prelude::*;

#[derive(Clone, Copy)]
pub(crate) struct CameraVectors {
    pub(crate) position: Vec3,
    pub(crate) target: Vec3,
    pub(crate) up: Vec3,
}

impl CameraVectors {
    pub(crate) fn from_renderer(renderer: &Renderer) -> Self {
        Self {
            position: renderer.camera_position(),
            target: renderer.camera_target(),
            up: renderer.camera_up(),
        }
    }
}

pub(crate) fn build_render_objects(world: &World, camera: CameraVectors) -> Vec<RenderObject> {
    let render_entities = collect_render_entities(world);

    render_entities
        .into_par_iter()
        .filter_map(|entity| prepare_render_object(camera, entity))
        .collect()
}

struct RenderEntity {
    mesh: Handle<Mesh>,
    material: Material,
    visible: bool,
    world_transform: Option<Transform>,
    local_transform: Option<Transform>,
    name: Option<String>,
    billboard: Option<Billboard>,
    depth_state: Option<DepthState>,
    gpu_instance: Option<GpuParticleInstance>,
}

fn collect_render_entities(world: &World) -> Vec<RenderEntity> {
    world
        .query::<(
            &MeshComponent,
            &MaterialComponent,
            &Visible,
            Option<&WorldTransform>,
            Option<&TransformComponent>,
            Option<&Name>,
            Option<&Billboard>,
            Option<&DepthState>,
            Option<&GpuParticleInstance>,
        )>()
        .iter()
        .map(
            |(
                _entity,
                (
                    mesh,
                    material,
                    visible,
                    world_transform,
                    local_transform,
                    name,
                    billboard,
                    depth_state,
                    gpu_instance,
                ),
            )| RenderEntity {
                mesh: mesh.0,
                material: material.0,
                visible: visible.0,
                world_transform: world_transform.map(|t| t.0),
                local_transform: local_transform.map(|t| t.0),
                name: name.map(|n| n.0.clone()),
                billboard: billboard.copied(),
                depth_state: depth_state.copied(),
                gpu_instance: gpu_instance.copied(),
            },
        )
        .collect()
}

fn prepare_render_object(camera: CameraVectors, entity: RenderEntity) -> Option<RenderObject> {
    if !entity.visible {
        return None;
    }

    let mut transform = select_render_transform(&entity);
    let mut material = entity.material;
    let billboard = entity.billboard;

    let instance_source = if entity.gpu_instance.is_some() {
        InstanceSource::Gpu
    } else {
        InstanceSource::Cpu
    };
    let gpu_index = entity.gpu_instance.map(|inst| inst.index);

    if let Some(billboard) = billboard {
        transform = apply_billboard_transform(
            transform,
            billboard,
            camera.position,
            camera.target,
            camera.up,
        );

        material = if billboard.lit {
            material.with_lit()
        } else {
            material.with_unlit()
        };
    }

    let depth_state = entity.depth_state.unwrap_or_default();
    let force_overlay = billboard.is_some() && !depth_state.depth_test && !depth_state.depth_write;

    Some(RenderObject {
        mesh: entity.mesh,
        material,
        transform,
        depth_state,
        force_overlay,
        instance_source,
        gpu_index,
    })
}

fn select_render_transform(entity: &RenderEntity) -> Transform {
    if let Some(world) = entity.world_transform {
        world
    } else if let Some(local) = entity.local_transform {
        if cfg!(debug_assertions) {
            if let Some(name) = &entity.name {
                log::warn!(
                    "Entity '{}' using LOCAL transform (no WorldTransform)",
                    name
                );
            }
        }
        local
    } else {
        Transform::IDENTITY
    }
}

pub(crate) fn apply_billboard_transform(
    transform: Transform,
    billboard: Billboard,
    camera_position: Vec3,
    camera_target: Vec3,
    camera_up: Vec3,
) -> Transform {
    let mut result = transform;

    let (view_right, view_up, view_forward) =
        build_view_basis(camera_position, camera_target, camera_up);

    let translation = match billboard.space {
        BillboardSpace::World => transform.translation,
        BillboardSpace::View { offset } => {
            camera_position + view_right * offset.x + view_up * offset.y + view_forward * offset.z
        }
    };

    let rotation_matrix = match billboard.space {
        BillboardSpace::View { .. } => Mat3::from_cols(view_right, view_up, -view_forward),
        BillboardSpace::World => billboard_world_matrix(
            billboard.orientation,
            translation,
            camera_position,
            camera_up,
        ),
    };

    let billboard_rotation = Quat::from_mat3(&rotation_matrix);
    result.translation = translation;
    result.rotation = billboard_rotation;
    result
}

fn build_view_basis(
    camera_position: Vec3,
    camera_target: Vec3,
    camera_up: Vec3,
) -> (Vec3, Vec3, Vec3) {
    let view_forward = safe_normalize(camera_target - camera_position, Vec3::NEG_Z);
    let view_up_hint = safe_normalize(camera_up, Vec3::Y);
    let (view_right, view_up) = basis_from_forward_up(view_forward, view_up_hint);
    (view_right, view_up, view_forward)
}

fn billboard_world_matrix(
    orientation: BillboardOrientation,
    translation: Vec3,
    camera_position: Vec3,
    camera_up: Vec3,
) -> Mat3 {
    match orientation {
        BillboardOrientation::FaceCamera => {
            let forward = safe_normalize(camera_position - translation, Vec3::Z);
            let up_hint = safe_normalize(camera_up, Vec3::Y);
            let (right, up) = basis_from_up_forward(up_hint, forward);
            Mat3::from_cols(right, up, forward)
        }
        BillboardOrientation::FaceCameraYAxis => {
            let forward = safe_normalize(
                Vec3::new(
                    camera_position.x - translation.x,
                    0.0,
                    camera_position.z - translation.z,
                ),
                Vec3::Z,
            );
            let (right, up) = basis_from_up_forward(Vec3::Y, forward);
            Mat3::from_cols(right, up, forward)
        }
    }
}

fn basis_from_forward_up(forward: Vec3, up_hint: Vec3) -> (Vec3, Vec3) {
    let mut right = forward.cross(up_hint);
    if right.length_squared() < 1e-6 {
        right = Vec3::X;
    } else {
        right = right.normalize();
    }

    let mut up = right.cross(forward);
    if up.length_squared() < 1e-6 {
        up = Vec3::Y;
    } else {
        up = up.normalize();
    }

    (right, up)
}

fn basis_from_up_forward(up_hint: Vec3, forward: Vec3) -> (Vec3, Vec3) {
    let mut right = up_hint.cross(forward);
    if right.length_squared() < 1e-6 {
        right = Vec3::X;
    } else {
        right = right.normalize();
    }

    let mut up = forward.cross(right);
    if up.length_squared() < 1e-6 {
        up = Vec3::Y;
    } else {
        up = up.normalize();
    }

    (right, up)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::transform::Transform;

    #[test]
    fn view_space_billboard_aligns_with_camera_basis() {
        let transform = Transform::IDENTITY;
        let offset = Vec3::new(2.0, -1.0, 6.0);
        let billboard = Billboard::new(BillboardOrientation::FaceCamera)
            .with_space(BillboardSpace::View { offset });
        let camera_pos = Vec3::new(1.5, -3.2, 4.0);
        let camera_target = Vec3::new(1.5, -2.2, -1.0);
        let camera_up = Vec3::new(0.0, 1.0, 0.1);

        let result =
            apply_billboard_transform(transform, billboard, camera_pos, camera_target, camera_up);

        let view_forward = safe_normalize(camera_target - camera_pos, Vec3::NEG_Z);
        let mut view_up = safe_normalize(camera_up, Vec3::Y);
        let mut view_right = view_forward.cross(view_up);
        if view_right.length_squared() < 1e-6 {
            view_right = Vec3::X;
        } else {
            view_right = view_right.normalize();
        }
        view_up = view_right.cross(view_forward);
        if view_up.length_squared() < 1e-6 {
            view_up = Vec3::Y;
        } else {
            view_up = view_up.normalize();
        }

        let expected_translation =
            camera_pos + view_right * offset.x + view_up * offset.y + view_forward * offset.z;

        assert!(result.translation.abs_diff_eq(expected_translation, 1e-5));
        assert!((result.rotation * Vec3::X).abs_diff_eq(view_right, 1e-5));
        assert!((result.rotation * Vec3::Y).abs_diff_eq(view_up, 1e-5));
        assert!((result.rotation * Vec3::Z).abs_diff_eq(-view_forward, 1e-5));
    }

    #[test]
    fn world_space_billboard_faces_camera_position() {
        let transform =
            Transform::from_trs(Vec3::new(-2.0, 1.0, -5.0), Quat::IDENTITY, Vec3::splat(1.5));
        let billboard = Billboard::new(BillboardOrientation::FaceCamera);
        let camera_pos = Vec3::new(4.0, 3.0, 2.0);
        let camera_target = Vec3::new(0.0, 0.0, 0.0);
        let camera_up = Vec3::Y;

        let result =
            apply_billboard_transform(transform, billboard, camera_pos, camera_target, camera_up);

        let expected_forward = safe_normalize(camera_pos - transform.translation, Vec3::Z);

        assert!(result.translation.abs_diff_eq(transform.translation, 1e-5));
        assert!((result.rotation * Vec3::Z).abs_diff_eq(expected_forward, 1e-5));
    }
}
