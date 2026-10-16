use std::collections::HashMap;

use bytemuck::cast_slice;

use super::{SceneImportDevice, SceneLoadContext};
use crate::asset::{Handle, MaterialAsset, Mesh, MeshData};
use crate::renderer::{SkinVertex, Vertex};
use crate::scene::components::{
    Children, GltfMaterial, GltfNode, GltfPrimitive, GltfSource, MaterialComponent, MeshBounds,
    MeshComponent, Name, Parent, TransformComponent, Visible,
};
use crate::scene::skeletal::{node_name, Animator, SkinnedMesh, MAX_JOINTS};
use crate::scene::{Scene, Transform};
use glam::Mat4;
use std::sync::Arc;

pub(super) struct MeshBuildResult {
    pub mesh_primitives: Vec<Vec<PrimitiveMesh>>,
}

#[derive(Clone, Copy)]
pub(super) struct PrimitiveMesh {
    pub handle: Handle<Mesh>,
    pub material_index: Option<usize>,
    pub bounds: MeshBounds,
    pub primitive_index: usize,
    /// Highest joint index the primitive's skin stream uses, if skinned.
    pub max_joint: Option<u16>,
}

pub(super) fn build_meshes<D: SceneImportDevice>(
    ctx: &mut SceneLoadContext<'_, D>,
    document: &gltf::Document,
    buffers: &[gltf::buffer::Data],
) -> Result<MeshBuildResult, String> {
    log::info!("Loading meshes...");
    let mesh_count = document.meshes().len();
    let mut mesh_primitives: Vec<Vec<PrimitiveMesh>> = vec![Vec::new(); mesh_count];
    let mut mesh_cache: MeshCache = HashMap::new();

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
            let (handle, bounds, max_joint) = {
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
                max_joint,
            });
        }
    }

    log::info!("Loaded {} meshes", mesh_count);

    Ok(MeshBuildResult { mesh_primitives })
}

pub(super) fn instantiate_nodes<D: SceneImportDevice>(
    ctx: &mut SceneLoadContext<'_, D>,
    document: &gltf::Document,
    buffers: &[gltf::buffer::Data],
    mesh_handles: &[Vec<PrimitiveMesh>],
    materials: &[Handle<MaterialAsset>],
    default_material: Handle<MaterialAsset>,
) -> Result<Vec<Option<hecs::Entity>>, String> {
    let mut node_entities: Vec<Option<hecs::Entity>> = vec![None; document.nodes().len()];
    // Entities drawing each node's mesh: the node's own and one per extra primitive.
    let mut mesh_entities: HashMap<usize, Vec<hecs::Entity>> = HashMap::new();

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
                &mut mesh_entities,
            )?;
        }
    }

    attach_skins(
        ctx,
        document,
        buffers,
        mesh_handles,
        &node_entities,
        &mesh_entities,
    )?;

    Ok(node_entities)
}

/// Give every mesh entity of a skinned node its [`SkinnedMesh`], once all joint
/// entities exist.
fn attach_skins<D: SceneImportDevice>(
    ctx: &mut SceneLoadContext<'_, D>,
    document: &gltf::Document,
    buffers: &[gltf::buffer::Data],
    mesh_handles: &[Vec<PrimitiveMesh>],
    node_entities: &[Option<hecs::Entity>],
    mesh_entities: &HashMap<usize, Vec<hecs::Entity>>,
) -> Result<(), String> {
    let mut skins: HashMap<usize, (Vec<hecs::Entity>, Arc<[Mat4]>)> = HashMap::new();
    for node in document.nodes() {
        let Some(skin) = node.skin() else {
            continue;
        };
        if let std::collections::hash_map::Entry::Vacant(slot) = skins.entry(skin.index()) {
            let joints = skin
                .joints()
                .map(|joint| {
                    node_entities
                        .get(joint.index())
                        .copied()
                        .flatten()
                        .ok_or_else(|| {
                            format!(
                                "skin {} uses joint '{}', which is not in the loaded scene",
                                skin.index(),
                                node_name(&joint)
                            )
                        })
                })
                .collect::<Result<Vec<_>, String>>()?;
            if joints.len() > MAX_JOINTS {
                return Err(format!(
                    "skin {} has {} joints; at most {MAX_JOINTS} are supported",
                    skin.index(),
                    joints.len()
                ));
            }
            let reader = skin.reader(|buffer| buffers.get(buffer.index()).map(|d| &d.0[..]));
            let inverse_bind: Vec<Mat4> = match reader.read_inverse_bind_matrices() {
                Some(matrices) => matrices
                    .map(|m| scale_inverse_bind(Mat4::from_cols_array_2d(&m), ctx.scale))
                    .collect(),
                None => vec![Mat4::IDENTITY; joints.len()],
            };
            if inverse_bind.len() != joints.len() {
                return Err(format!(
                    "skin {} has {} joints but {} inverse bind matrices",
                    skin.index(),
                    joints.len(),
                    inverse_bind.len()
                ));
            }
            slot.insert((joints, Arc::from(inverse_bind)));
        }
        let (joints, inverse_bind) = &skins[&skin.index()];

        if let Some(primitives) = node.mesh().and_then(|mesh| mesh_handles.get(mesh.index())) {
            if let Some(max) = primitives.iter().filter_map(|p| p.max_joint).max() {
                if max as usize >= joints.len() {
                    log::warn!(
                        "Mesh of node '{}' uses joint {max} but its skin has {} joints",
                        node_name(&node),
                        joints.len()
                    );
                }
            }
        }
        for &entity in mesh_entities.get(&node.index()).into_iter().flatten() {
            let skinned = ctx
                .scene
                .assets
                .meshes
                .get(
                    ctx.scene
                        .world()
                        .get::<&MeshComponent>(entity)
                        .map_err(|e| e.to_string())?
                        .0,
                )
                .is_some_and(Mesh::is_skinned);
            if skinned {
                ctx.scene
                    .world_mut()
                    .insert_one(
                        entity,
                        SkinnedMesh::new(joints.clone(), Arc::clone(inverse_bind)),
                    )
                    .map_err(|err| err.to_string())?;
            } else {
                log::warn!(
                    "Node '{}' has a skin but its mesh has no JOINTS_0/WEIGHTS_0; drawing it rigid",
                    node_name(&node)
                );
            }
        }
    }
    Ok(())
}

/// glTF import scale multiplies positions and node translations, so the bind matrices
/// (mesh space to joint space) must be conjugated by it: their translation scales too.
pub(super) fn scale_inverse_bind(matrix: Mat4, scale: f32) -> Mat4 {
    let mut scaled = matrix;
    scaled.w_axis.x *= scale;
    scaled.w_axis.y *= scale;
    scaled.w_axis.z *= scale;
    scaled
}

/// Put an [`Animator`] on each root node whose subtree holds skin joints, so skinned
/// files play their animations by bone name (and can take clips from other files).
/// Returns whether any was added; the file's animations then need no entity-bound clips.
pub(super) fn attach_animators<D: SceneImportDevice>(
    ctx: &mut SceneLoadContext<'_, D>,
    document: &gltf::Document,
    buffers: &[gltf::buffer::Data],
    node_entities: &[Option<hecs::Entity>],
) -> bool {
    // A skinned file without animations (a character downloaded in T-pose) gets one
    // too, to take clips from other files.
    if document.skins().len() == 0 {
        return false;
    }
    let joint_nodes: std::collections::HashSet<usize> = document
        .skins()
        .flat_map(|skin| skin.joints().map(|joint| joint.index()).collect::<Vec<_>>())
        .collect();
    let default_clip =
        crate::scene::skeletal::ClipLibrary::from_document(ctx.source_path(), document, buffers)
            .clips
            .first()
            .map(|clip| clip.name.clone());

    let mut added = false;
    for scene in document.scenes() {
        for root in scene.nodes() {
            let mut stack = vec![root.clone()];
            let mut has_joint = false;
            while let Some(node) = stack.pop() {
                if joint_nodes.contains(&node.index()) {
                    has_joint = true;
                    break;
                }
                stack.extend(node.children());
            }
            let Some(entity) = has_joint
                .then(|| node_entities.get(root.index()).copied().flatten())
                .flatten()
            else {
                continue;
            };
            let animator = Animator {
                default_clip: default_clip.clone(),
                ..Animator::with_library(ctx.source_path().to_path_buf())
            };
            if ctx.scene.world_mut().insert_one(entity, animator).is_ok() {
                added = true;
            }
        }
    }
    added
}

#[allow(clippy::too_many_arguments)]
fn load_node<D: SceneImportDevice>(
    ctx: &mut SceneLoadContext<'_, D>,
    node: &gltf::Node,
    parent: Option<hecs::Entity>,
    mesh_handles: &[Vec<PrimitiveMesh>],
    materials: &[Handle<MaterialAsset>],
    default_material: Handle<MaterialAsset>,
    node_entities: &mut [Option<hecs::Entity>],
    mesh_entities: &mut HashMap<usize, Vec<hecs::Entity>>,
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
            // The same name a skeletal clip uses for an unnamed bone.
            node_name(node)
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
    if node.mesh().is_some() {
        mesh_entities.entry(node.index()).or_default().push(entity);
    }

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
        mesh_entities
            .entry(node.index())
            .or_default()
            .push(primitive_entity);
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
            mesh_entities,
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

/// Meshes built so far by content signature: handle, bounds and highest joint used.
type MeshCache = HashMap<Vec<u8>, (Handle<Mesh>, MeshBounds, Option<u16>)>;

/// One `JOINTS_n`/`WEIGHTS_n` attribute set.
type InfluenceSet = (Vec<[u16; 4]>, Vec<[f32; 4]>);

fn load_primitive(
    primitive: &gltf::Primitive,
    buffers: &[gltf::buffer::Data],
    scene: &mut Scene,
    renderer: &mut impl SceneImportDevice,
    scale_multiplier: f32,
    mesh_cache: &mut MeshCache,
) -> Result<(Handle<Mesh>, MeshBounds, Option<u16>), String> {
    let data = primitive_mesh_data(primitive, buffers, scale_multiplier)?;

    let bounds = MeshBounds::from_vertices(&data.vertices)
        .ok_or_else(|| "Primitive has no vertex positions".to_string())?;
    let max_joint = data.max_joint_index();

    // Meshes differing only in their weights must not collapse into one.
    let skin_bytes = data
        .skin
        .as_deref()
        .map_or(&[][..], |skin| cast_slice(skin));
    let mut signature = Vec::with_capacity(
        data.vertices.len() * std::mem::size_of::<Vertex>()
            + data.indices.len() * std::mem::size_of::<u32>()
            + skin_bytes.len(),
    );
    signature.extend_from_slice(cast_slice(&data.vertices));
    signature.extend_from_slice(cast_slice(&data.indices));
    signature.extend_from_slice(skin_bytes);

    if let Some(existing) = mesh_cache.get(&signature) {
        return Ok(*existing);
    }

    let mesh = renderer.create_mesh_from_data(data);
    let handle = scene.assets.meshes.insert(mesh);
    mesh_cache.insert(signature, (handle, bounds, max_joint));

    Ok((handle, bounds, max_joint))
}

/// The vertices, indices and (for skinned primitives) skin stream of one primitive,
/// with positions multiplied by `scale_multiplier`. Missing normals are computed from
/// the triangles, missing tangents from the UVs.
pub(super) fn primitive_mesh_data(
    primitive: &gltf::Primitive,
    buffers: &[gltf::buffer::Data],
    scale_multiplier: f32,
) -> Result<MeshData, String> {
    let reader = primitive.reader(|buffer| Some(&buffers[buffer.index()]));

    let positions = reader
        .read_positions()
        .ok_or("Missing positions")?
        .collect::<Vec<_>>();

    let indices = match reader.read_indices() {
        Some(indices) => indices.into_u32().collect::<Vec<_>>(),
        // Non-indexed triangle lists draw their vertices in order.
        None => (0..positions.len() as u32).collect(),
    };

    let normals = reader
        .read_normals()
        .map(|n| n.collect::<Vec<_>>())
        .filter(|n| n.len() == positions.len())
        .unwrap_or_else(|| {
            log::debug!("    No normals in glTF, computing smooth normals");
            generate_normals(&positions, &indices)
        });

    let uvs = reader
        .read_tex_coords(0)
        .map(|uv| uv.into_f32().collect::<Vec<_>>())
        .unwrap_or_else(|| vec![[0.0, 0.0]; positions.len()]);

    // glTF vertex colours are linear, like ours.
    let colors = reader
        .read_colors(0)
        .map(|c| c.into_rgba_f32().collect::<Vec<_>>())
        .filter(|c| c.len() == positions.len());

    let tangents = reader
        .read_tangents()
        .map(|t| t.collect::<Vec<_>>())
        .unwrap_or_else(|| {
            log::debug!("    No tangents in glTF, generating them");
            generate_tangents(&positions, &normals, &uvs, &indices)
        });

    let vertices = positions
        .iter()
        .zip(normals.iter())
        .zip(uvs.iter())
        .zip(tangents.iter())
        .enumerate()
        .map(|(i, (((pos, norm), uv), tangent))| {
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
                color: colors.as_ref().map_or(Vertex::WHITE, |c| c[i]),
            }
        })
        .collect::<Vec<_>>();

    let skin = read_skin_stream(&reader, positions.len());

    Ok(MeshData {
        vertices,
        indices,
        skin,
    })
}

/// Joint indices and weights from `JOINTS_0`/`WEIGHTS_0` (and a second set if present;
/// the four strongest influences are kept and renormalized).
fn read_skin_stream<'a, 's, F>(
    reader: &gltf::mesh::Reader<'a, 's, F>,
    vertex_count: usize,
) -> Option<Vec<SkinVertex>>
where
    F: Clone + Fn(gltf::Buffer<'a>) -> Option<&'s [u8]>,
{
    let mut sets: Vec<InfluenceSet> = Vec::new();
    for set in 0..2 {
        let (Some(joints), Some(weights)) = (reader.read_joints(set), reader.read_weights(set))
        else {
            break;
        };
        let joints: Vec<[u16; 4]> = joints.into_u16().collect();
        let weights: Vec<[f32; 4]> = weights.into_f32().collect();
        if joints.len() != vertex_count || weights.len() != vertex_count {
            log::warn!("Skin attribute set {set} does not match the vertex count; ignored");
            break;
        }
        sets.push((joints, weights));
    }
    if sets.is_empty() {
        return None;
    }
    if sets.len() > 1 {
        log::debug!("    Primitive has 8 joint influences; keeping the strongest 4");
    }
    Some(
        (0..vertex_count)
            .map(|vertex| {
                SkinVertex::from_influences(
                    sets.iter().flat_map(|(joints, weights)| {
                        joints[vertex].into_iter().zip(weights[vertex])
                    }),
                )
            })
            .collect(),
    )
}

/// Area-weighted smooth normals for meshes that come without any.
fn generate_normals(positions: &[[f32; 3]], indices: &[u32]) -> Vec<[f32; 3]> {
    use glam::Vec3;
    let mut normals = vec![Vec3::ZERO; positions.len()];
    for triangle in indices.as_chunks::<3>().0 {
        let [a, b, c] = triangle.map(|i| i as usize);
        if a >= positions.len() || b >= positions.len() || c >= positions.len() {
            continue;
        }
        let (pa, pb, pc) = (
            Vec3::from(positions[a]),
            Vec3::from(positions[b]),
            Vec3::from(positions[c]),
        );
        // Unnormalized: larger triangles weigh more.
        let face = (pb - pa).cross(pc - pa);
        normals[a] += face;
        normals[b] += face;
        normals[c] += face;
    }
    normals
        .into_iter()
        .map(|n| n.try_normalize().unwrap_or(Vec3::Y).to_array())
        .collect()
}

fn generate_tangents(
    positions: &[[f32; 3]],
    normals: &[[f32; 3]],
    uvs: &[[f32; 2]],
    indices: &[u32],
) -> Vec<[f32; 4]> {
    use glam::{Vec2, Vec3};

    let vertex_count = positions.len();
    let mut tangents = vec![Vec3::ZERO; vertex_count];
    let mut bitangents = vec![Vec3::ZERO; vertex_count];

    for triangle in indices.chunks(3) {
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
