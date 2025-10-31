use std::collections::HashMap;

use bytemuck::cast_slice;

use super::{SceneImportDevice, SceneLoadContext};
use crate::asset::{Handle, MaterialAsset, Mesh};
use crate::renderer::Vertex;
use crate::scene::components::{
    Children, GltfMaterial, GltfNode, GltfPrimitive, GltfSource, MaterialComponent, MeshBounds,
    MeshComponent, Name, Parent, TransformComponent, Visible,
};
use crate::scene::{Scene, Transform};

pub(super) struct MeshBuildResult {
    pub mesh_primitives: Vec<Vec<PrimitiveMesh>>,
}

#[derive(Clone, Copy)]
pub(super) struct PrimitiveMesh {
    pub handle: Handle<Mesh>,
    pub material_index: Option<usize>,
    pub bounds: MeshBounds,
    pub primitive_index: usize,
}

pub(super) fn build_meshes<D: SceneImportDevice>(
    ctx: &mut SceneLoadContext<'_, D>,
    document: &gltf::Document,
    buffers: &[gltf::buffer::Data],
) -> Result<MeshBuildResult, String> {
    log::info!("Loading meshes...");
    let mesh_count = document.meshes().len();
    let mut mesh_primitives: Vec<Vec<PrimitiveMesh>> = vec![Vec::new(); mesh_count];
    let mut mesh_cache: HashMap<Vec<u8>, (Handle<Mesh>, MeshBounds)> = HashMap::new();

    for gltf_mesh in document.meshes() {
        let mesh_index = gltf_mesh.index();
        let mesh_name = gltf_mesh.name().unwrap_or("Unnamed");
        let primitive_count = gltf_mesh.primitives().len();

        log::debug!(
            "  Mesh {}: '{}' with {} primitives",
            mesh_index,
            mesh_name,
            primitive_count
        );

        for (primitive_index, primitive) in gltf_mesh.primitives().enumerate() {
            let (handle, bounds) = {
                let scene = &mut *ctx.scene;
                let renderer = &mut *ctx.renderer;
                load_primitive(
                    &primitive,
                    buffers,
                    scene,
                    renderer,
                    ctx.scale,
                    &mut mesh_cache,
                )?
            };

            mesh_primitives[mesh_index].push(PrimitiveMesh {
                handle,
                material_index: primitive.material().index(),
                bounds,
                primitive_index,
            });
        }
    }

    log::info!("Loaded {} meshes", mesh_count);

    Ok(MeshBuildResult { mesh_primitives })
}

pub(super) fn instantiate_nodes<D: SceneImportDevice>(
    ctx: &mut SceneLoadContext<'_, D>,
    document: &gltf::Document,
    mesh_handles: &[Vec<PrimitiveMesh>],
    materials: &[Handle<MaterialAsset>],
    default_material: Handle<MaterialAsset>,
) -> Result<Vec<Option<hecs::Entity>>, String> {
    let mut node_entities: Vec<Option<hecs::Entity>> = vec![None; document.nodes().len()];

    log::info!("Loading scene hierarchies...");
    for (scene_index, gltf_scene) in document.scenes().enumerate() {
        let scene_name = gltf_scene.name().unwrap_or("Unnamed");
        let root_count = gltf_scene.nodes().len();

        log::info!(
            "  Scene {}: '{}' with {} root nodes (scale: {}x)",
            scene_index,
            scene_name,
            root_count,
            ctx.scale
        );

        for (node_index, node) in gltf_scene.nodes().enumerate() {
            log::info!(
                "    Loading root node {}/{}: {:?}",
                node_index + 1,
                root_count,
                node.name()
            );

            load_node(
                ctx,
                &node,
                None,
                mesh_handles,
                materials,
                default_material,
                &mut node_entities,
            )?;
        }
    }

    Ok(node_entities)
}

fn load_node<D: SceneImportDevice>(
    ctx: &mut SceneLoadContext<'_, D>,
    node: &gltf::Node,
    parent: Option<hecs::Entity>,
    mesh_handles: &[Vec<PrimitiveMesh>],
    materials: &[Handle<MaterialAsset>],
    default_material: Handle<MaterialAsset>,
    node_entities: &mut [Option<hecs::Entity>],
) -> Result<hecs::Entity, String> {
    let source_path = ctx.source_path().to_path_buf();
    let node_name = node.name().map(|name| name.to_string()).unwrap_or_else(|| {
        if parent.is_none() {
            source_path
                .file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| "Imported glTF".to_string())
        } else {
            "Unnamed".to_string()
        }
    });

    log::debug!(
        "Loading node: {} (index: {}, parent: {:?})",
        node_name,
        node.index(),
        parent
    );

    let (translation, rotation, scale) = node.transform().decomposed();
    let mut transform = Transform {
        translation: glam::Vec3::from(translation),
        rotation: glam::Quat::from_array(rotation),
        scale: glam::Vec3::from(scale),
    };
    transform.translation *= ctx.scale;

    let mut entity_builder = hecs::EntityBuilder::new();
    entity_builder.add(Name::new(node_name.clone()));
    entity_builder.add(TransformComponent(transform));
    entity_builder.add(Visible(true));
    entity_builder.add(GltfNode(node.index()));
    entity_builder.add(GltfSource(source_path.clone()));

    if let Some(parent_entity) = parent {
        entity_builder.add(Parent(parent_entity));
    }

    let mut extra_primitives: Vec<PrimitiveMesh> = Vec::new();

    if let Some(gltf_mesh) = node.mesh() {
        if let Some(primitives) = mesh_handles.get(gltf_mesh.index()) {
            if let Some(primary) = primitives.first().copied() {
                entity_builder.add(MeshComponent(primary.handle));
                entity_builder.add(primary.bounds);
                entity_builder.add(GltfPrimitive(primary.primitive_index));

                let material_handle = primary
                    .material_index
                    .and_then(|mat_idx| materials.get(mat_idx).copied())
                    .unwrap_or(default_material);
                entity_builder.add(MaterialComponent(material_handle));

                if let Some(mat_idx) = primary.material_index {
                    entity_builder.add(GltfMaterial(mat_idx));
                }

                if primitives.len() > 1 {
                    extra_primitives.extend_from_slice(&primitives[1..]);
                }
            }
        }
    }

    let entity = {
        let world = ctx.scene.world_mut();
        let entity = world.spawn(entity_builder.build());
        if let Some(slot) = node_entities.get_mut(node.index()) {
            *slot = Some(entity);
        }
        entity
    };

    let mut children = Vec::new();

    for (primitive_slot, primitive) in extra_primitives.into_iter().enumerate() {
        let primitive_name = format!("{}_Primitive_{}", node_name, primitive_slot + 1);

        let mut primitive_builder = hecs::EntityBuilder::new();
        primitive_builder.add(Name::new(primitive_name));
        primitive_builder.add(TransformComponent(Transform::IDENTITY));
        primitive_builder.add(Visible(true));
        primitive_builder.add(Parent(entity));
        primitive_builder.add(GltfNode(node.index()));
        primitive_builder.add(GltfSource(source_path.clone()));
        primitive_builder.add(MeshComponent(primitive.handle));
        primitive_builder.add(primitive.bounds);
        primitive_builder.add(GltfPrimitive(primitive.primitive_index));

        let material_handle = primitive
            .material_index
            .and_then(|mat_idx| materials.get(mat_idx).copied())
            .unwrap_or(default_material);
        primitive_builder.add(MaterialComponent(material_handle));

        if let Some(mat_idx) = primitive.material_index {
            primitive_builder.add(GltfMaterial(mat_idx));
        }

        let primitive_entity = ctx.scene.world_mut().spawn(primitive_builder.build());
        children.push(primitive_entity);
    }

    for child_node in node.children() {
        let child_entity = load_node(
            ctx,
            &child_node,
            Some(entity),
            mesh_handles,
            materials,
            default_material,
            node_entities,
        )?;
        children.push(child_entity);
    }

    if !children.is_empty() {
        ctx.scene
            .world_mut()
            .insert_one(entity, Children(children))
            .ok();
    }

    Ok(entity)
}

fn load_primitive(
    primitive: &gltf::Primitive,
    buffers: &[gltf::buffer::Data],
    scene: &mut Scene,
    renderer: &mut impl SceneImportDevice,
    scale_multiplier: f32,
    mesh_cache: &mut HashMap<Vec<u8>, (Handle<Mesh>, MeshBounds)>,
) -> Result<(Handle<Mesh>, MeshBounds), String> {
    let reader = primitive.reader(|buffer| Some(&buffers[buffer.index()]));

    let positions = reader
        .read_positions()
        .ok_or("Missing positions")?
        .collect::<Vec<_>>();

    let normals = reader
        .read_normals()
        .map(|n| n.collect::<Vec<_>>())
        .unwrap_or_else(|| vec![[0.0, 1.0, 0.0]; positions.len()]);

    let uvs = reader
        .read_tex_coords(0)
        .map(|uv| uv.into_f32().collect::<Vec<_>>())
        .unwrap_or_else(|| vec![[0.0, 0.0]; positions.len()]);

    let tangents = reader
        .read_tangents()
        .map(|t| t.collect::<Vec<_>>())
        .unwrap_or_else(|| {
            log::debug!("    No tangents in glTF, generating them");
            generate_tangents(&positions, &normals, &uvs, &reader.read_indices())
        });

    let indices = reader
        .read_indices()
        .ok_or("Missing indices")?
        .into_u32()
        .collect::<Vec<_>>();

    let vertices = positions
        .iter()
        .zip(normals.iter())
        .zip(uvs.iter())
        .zip(tangents.iter())
        .map(|(((pos, norm), uv), tangent)| {
            let scaled_pos = [
                pos[0] * scale_multiplier,
                pos[1] * scale_multiplier,
                pos[2] * scale_multiplier,
            ];

            Vertex {
                pos: scaled_pos,
                normal: *norm,
                uv: *uv,
                tangent: *tangent,
            }
        })
        .collect::<Vec<_>>();

    let bounds = MeshBounds::from_vertices(&vertices)
        .ok_or_else(|| "Primitive has no vertex positions".to_string())?;

    let mut signature = Vec::with_capacity(
        vertices.len() * std::mem::size_of::<Vertex>() + indices.len() * std::mem::size_of::<u32>(),
    );
    signature.extend_from_slice(cast_slice(&vertices));
    signature.extend_from_slice(cast_slice(&indices));

    if let Some(existing) = mesh_cache.get(&signature) {
        return Ok(*existing);
    }

    let mesh = renderer.create_mesh(&vertices, &indices);
    let handle = scene.assets.meshes.insert(mesh);
    mesh_cache.insert(signature, (handle, bounds));

    Ok((handle, bounds))
}

fn generate_tangents(
    positions: &[[f32; 3]],
    normals: &[[f32; 3]],
    uvs: &[[f32; 2]],
    indices: &Option<gltf::mesh::util::ReadIndices>,
) -> Vec<[f32; 4]> {
    use glam::{Vec2, Vec3};

    let vertex_count = positions.len();
    let mut tangents = vec![Vec3::ZERO; vertex_count];
    let mut bitangents = vec![Vec3::ZERO; vertex_count];

    let index_iter: Vec<u32> = if let Some(idx) = indices {
        idx.clone().into_u32().collect()
    } else {
        (0..vertex_count as u32).collect()
    };

    for triangle in index_iter.chunks(3) {
        if triangle.len() != 3 {
            continue;
        }

        let i0 = triangle[0] as usize;
        let i1 = triangle[1] as usize;
        let i2 = triangle[2] as usize;

        let p0 = Vec3::from(positions[i0]);
        let p1 = Vec3::from(positions[i1]);
        let p2 = Vec3::from(positions[i2]);

        let uv0 = Vec2::from(uvs[i0]);
        let uv1 = Vec2::from(uvs[i1]);
        let uv2 = Vec2::from(uvs[i2]);

        let edge1 = p1 - p0;
        let edge2 = p2 - p0;
        let delta_uv1 = uv1 - uv0;
        let delta_uv2 = uv2 - uv0;

        let f = 1.0 / (delta_uv1.x * delta_uv2.y - delta_uv2.x * delta_uv1.y);

        let tangent = if f.is_finite() {
            Vec3::new(
                f * (delta_uv2.y * edge1.x - delta_uv1.y * edge2.x),
                f * (delta_uv2.y * edge1.y - delta_uv1.y * edge2.y),
                f * (delta_uv2.y * edge1.z - delta_uv1.y * edge2.z),
            )
        } else {
            Vec3::X
        };

        let bitangent = if f.is_finite() {
            Vec3::new(
                f * (-delta_uv2.x * edge1.x + delta_uv1.x * edge2.x),
                f * (-delta_uv2.x * edge1.y + delta_uv1.x * edge2.y),
                f * (-delta_uv2.x * edge1.z + delta_uv1.x * edge2.z),
            )
        } else {
            Vec3::Y
        };

        tangents[i0] += tangent;
        tangents[i1] += tangent;
        tangents[i2] += tangent;

        bitangents[i0] += bitangent;
        bitangents[i1] += bitangent;
        bitangents[i2] += bitangent;
    }

    tangents
        .iter()
        .zip(bitangents.iter())
        .zip(normals.iter())
        .map(|((t, b), n)| {
            let normal = Vec3::from(*n);
            let mut tangent = *t;

            tangent = (tangent - normal * normal.dot(tangent)).normalize_or_zero();

            if tangent.length_squared() < 0.0001 {
                tangent = if normal.y.abs() < 0.999 {
                    Vec3::Y.cross(normal).normalize()
                } else {
                    Vec3::X.cross(normal).normalize()
                };
            }

            let bitangent = *b;
            let handedness = if normal.cross(tangent).dot(bitangent) < 0.0 {
                -1.0
            } else {
                1.0
            };

            [tangent.x, tangent.y, tangent.z, handedness]
        })
        .collect()
}
