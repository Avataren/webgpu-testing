// scene/loader.rs - Improved version with better debugging
use glam::{Quat, Vec3};
use std::path::Path;

use super::components::*;
use crate::asset::Handle;
use crate::asset::Mesh;
use crate::renderer::{Material, Renderer, Texture, Vertex};
use crate::scene::{Scene, Transform};

pub struct SceneLoader;

impl SceneLoader {
    fn load_node(
        node: &gltf::Node,
        parent: Option<hecs::Entity>,
        mesh_handles: &[Vec<(Handle<Mesh>, Option<usize>)>],
        materials: &[Material],
        world: &mut hecs::World,
        scale_multiplier: f32,
    ) -> Result<hecs::Entity, String> {
        let node_name = node.name().unwrap_or("Unnamed");
        log::debug!(
            "Loading node: {} (index: {}, parent: {:?})",
            node_name,
            node.index(),
            parent
        );

        // Get transform from glTF
        let (translation, rotation, scale) = node.transform().decomposed();
        let mut transform = Transform {
            translation: Vec3::from(translation),
            rotation: Quat::from_array(rotation),
            scale: Vec3::from(scale),
        };

        // Apply scale multiplier to convert units. We only scale translations here; scaling the
        // local scale at every level breaks hierarchical transforms because the multiplier would
        // be applied once per parent. Mesh vertex data is scaled uniformly when loaded instead.
        transform.translation *= scale_multiplier;

        log::debug!(
            "  Transform: T={:?}, R={:?}, S={:?}",
            transform.translation,
            transform.rotation,
            transform.scale
        );

        // Build entity using pure hecs
        let mut entity_builder = hecs::EntityBuilder::new();

        // Add core components
        entity_builder.add(Name::new(node_name));
        entity_builder.add(TransformComponent(transform));
        entity_builder.add(Visible(true));

        // Add parent if exists
        if let Some(parent_entity) = parent {
            entity_builder.add(Parent(parent_entity));
            log::debug!("  Has parent: {:?}", parent_entity);
        } else {
            log::debug!("  Root node (no parent)");
        }

        // Handle mesh primitives
        // The first primitive is added to this entity
        // Additional primitives become child entities
        let mut extra_primitives: Vec<(Handle<Mesh>, Option<usize>)> = Vec::new();

        if let Some(gltf_mesh) = node.mesh() {
            log::debug!(
                "  Has mesh with {} primitives",
                gltf_mesh.primitives().len()
            );

            if let Some(primitives) = mesh_handles.get(gltf_mesh.index()) {
                if !primitives.is_empty() {
                    // Add first primitive to this entity
                    let (mesh_handle, material_index) = primitives[0];
                    entity_builder.add(MeshComponent(mesh_handle));

                    let material = if let Some(mat_idx) = material_index {
                        materials.get(mat_idx).copied().unwrap_or(Material::pbr())
                    } else {
                        Material::pbr()
                    };
                    entity_builder.add(MaterialComponent(material));

                    log::debug!("  Added primary mesh primitive");

                    // Store remaining primitives
                    if primitives.len() > 1 {
                        extra_primitives.extend(primitives[1..].iter().copied());
                        log::debug!("  Has {} extra primitives", extra_primitives.len());
                    }
                }
            }
        } else {
            log::debug!("  No mesh (transform-only node)");
        }

        // Spawn the entity
        let entity = world.spawn(entity_builder.build());
        log::debug!("  Spawned entity: {:?}", entity);

        // Track all children
        let mut children = Vec::new();

        // Spawn extra mesh primitives as child entities
        for (primitive_index, (mesh_handle, material_index)) in
            extra_primitives.into_iter().enumerate()
        {
            let primitive_name = format!("{}_Primitive_{}", node_name, primitive_index + 1);
            log::debug!("  Creating extra primitive: {}", primitive_name);

            let mut primitive_builder = hecs::EntityBuilder::new();
            primitive_builder.add(Name::new(primitive_name));

            // Identity transform - shares parent's transform
            primitive_builder.add(TransformComponent(Transform::IDENTITY));
            primitive_builder.add(Visible(true));
            primitive_builder.add(Parent(entity));
            primitive_builder.add(MeshComponent(mesh_handle));

            let material = if let Some(mat_idx) = material_index {
                materials.get(mat_idx).copied().unwrap_or(Material::pbr())
            } else {
                Material::pbr()
            };
            primitive_builder.add(MaterialComponent(material));

            let primitive_entity = world.spawn(primitive_builder.build());
            children.push(primitive_entity);
            log::debug!("    Primitive entity: {:?}", primitive_entity);
        }

        // Recursively load child nodes
        log::debug!("  Processing {} child nodes", node.children().count());
        for child_node in node.children() {
            let child_entity = Self::load_node(
                &child_node,
                Some(entity),
                mesh_handles,
                materials,
                world,
                scale_multiplier,
            )?;
            children.push(child_entity);
        }

        // Add Children component if we have any
        if !children.is_empty() {
            log::debug!(
                "  Adding {} children to entity {:?}",
                children.len(),
                entity
            );
            world.insert_one(entity, Children(children)).ok();
        }

        Ok(entity)
    }

    /// Load a glTF file into the scene with scale
    pub fn load_gltf(
        path: impl AsRef<Path>,
        scene: &mut Scene,
        renderer: &mut Renderer,
        scale: f32,
    ) -> Result<(), String> {
        let path = path.as_ref();
        log::info!("=== Loading glTF: {:?} ===", path);

        #[cfg(target_arch = "wasm32")]
        let (document, buffers, images) =
            Self::import_gltf_web(path).map_err(|e| format!("Failed to load glTF: {}", e))?;

        #[cfg(not(target_arch = "wasm32"))]
        let (document, buffers, images) =
            gltf::import(path).map_err(|e| format!("Failed to load glTF: {}", e))?;

        log::info!(
            "Document info: {} meshes, {} materials, {} textures, {} scenes",
            document.meshes().len(),
            document.materials().len(),
            document.textures().len(),
            document.scenes().len()
        );

        // Get the base directory for loading external textures
        let base_dir = path.parent().unwrap_or_else(|| Path::new("."));

        // Load all textures first
        log::info!("Loading textures...");
        let texture_handles = Self::load_textures(&document, &images, base_dir, scene, renderer)?;
        log::info!("Loaded {} textures", texture_handles.len());

        // Load all materials
        log::info!("Loading materials...");
        let material_handles = Self::load_materials(&document, &texture_handles)?;
        log::info!("Loaded {} materials", material_handles.len());

        // Load all meshes (each mesh can have multiple primitives)
        log::info!("Loading meshes...");
        let mesh_count = document.meshes().len();
        let mut mesh_handles: Vec<Vec<(Handle<Mesh>, Option<usize>)>> =
            vec![Vec::new(); mesh_count];

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

            let primitives = &mut mesh_handles[mesh_index];

            for primitive in gltf_mesh.primitives() {
                let handle = Self::load_primitive(&primitive, &buffers, scene, renderer, scale)?;
                primitives.push((handle, primitive.material().index()));
            }
        }
        log::info!("Loaded {} meshes", mesh_count);

        // Load all scenes and their node hierarchies
        log::info!("Loading scene hierarchies...");
        for (scene_index, gltf_scene) in document.scenes().enumerate() {
            let scene_name = gltf_scene.name().unwrap_or("Unnamed");
            let root_count = gltf_scene.nodes().len();

            log::info!(
                "  Scene {}: '{}' with {} root nodes (scale: {}x)",
                scene_index,
                scene_name,
                root_count,
                scale
            );

            for (node_index, node) in gltf_scene.nodes().enumerate() {
                log::info!(
                    "    Loading root node {}/{}: {:?}",
                    node_index + 1,
                    root_count,
                    node.name()
                );

                Self::load_node(
                    &node,
                    None,
                    &mesh_handles,
                    &material_handles,
                    &mut scene.world,
                    scale,
                )?;
            }
        }

        log::info!("=== glTF loaded successfully ===");
        log::info!("Total entities in scene: {}", scene.world.len());

        // Count entities with different components
        let mesh_count = scene.world.query::<&MeshComponent>().iter().count();
        let parent_count = scene.world.query::<&Parent>().iter().count();
        let children_count = scene.world.query::<&Children>().iter().count();

        log::info!("  Entities with meshes: {}", mesh_count);
        log::info!("  Entities with parent: {}", parent_count);
        log::info!("  Entities with children: {}", children_count);

        Ok(())
    }

    /// Load all textures from glTF
    fn load_textures(
        document: &gltf::Document,
        images: &[gltf::image::Data],
        base_dir: &Path,
        scene: &mut Scene,
        renderer: &mut Renderer,
    ) -> Result<Vec<u32>, String> {
        let mut handles = Vec::new();

        for gltf_texture in document.textures() {
            let source = gltf_texture.source();
            let texture = match source.source() {
                gltf::image::Source::Uri { uri, .. } => {
                    let texture_path = base_dir.join(uri);
                    log::debug!("  Loading texture from file: {:?}", texture_path);

                    Texture::from_path(
                        renderer.get_device(),
                        renderer.get_queue(),
                        &texture_path,
                        false, // sRGB
                    )?
                }
                gltf::image::Source::View { .. } => {
                    let img_data = &images[source.index()];
                    log::debug!(
                        "  Loading embedded texture: {}x{}",
                        img_data.width,
                        img_data.height
                    );

                    Texture::from_bytes(
                        renderer.get_device(),
                        renderer.get_queue(),
                        &img_data.pixels,
                        img_data.width,
                        img_data.height,
                        Some(&format!("EmbeddedTexture_{}", source.index())),
                    )
                }
            };

            let handle = scene.assets.textures.insert(texture);
            handles.push(handle.index() as u32);
        }

        Ok(handles)
    }

    /// Load all materials from glTF
    fn load_materials(
        document: &gltf::Document,
        texture_handles: &[u32],
    ) -> Result<Vec<Material>, String> {
        let mut materials = Vec::new();

        for gltf_mat in document.materials() {
            let mat_name = gltf_mat.name().unwrap_or("Unnamed");
            let pbr = gltf_mat.pbr_metallic_roughness();

            // Base color
            let base_color = pbr.base_color_factor();
            let base_color_u8 = [
                (base_color[0] * 255.0) as u8,
                (base_color[1] * 255.0) as u8,
                (base_color[2] * 255.0) as u8,
                (base_color[3] * 255.0) as u8,
            ];

            let mut material = Material::new(base_color_u8)
                .with_metallic(pbr.metallic_factor())
                .with_roughness(pbr.roughness_factor());

            // Base color texture
            if let Some(info) = pbr.base_color_texture() {
                let tex_index = info.texture().index();
                if tex_index < texture_handles.len() {
                    material = material.with_base_color_texture(texture_handles[tex_index]);
                }
            }

            // Metallic-roughness texture
            if let Some(info) = pbr.metallic_roughness_texture() {
                let tex_index = info.texture().index();
                if tex_index < texture_handles.len() {
                    material = material.with_metallic_roughness_texture(texture_handles[tex_index]);
                }
            }

            // Normal map
            if let Some(normal) = gltf_mat.normal_texture() {
                let tex_index = normal.texture().index();
                if tex_index < texture_handles.len() {
                    material = material.with_normal_texture(texture_handles[tex_index]);
                }
            }

            // Emissive
            if let Some(emissive) = gltf_mat.emissive_texture() {
                let tex_index = emissive.texture().index();
                if tex_index < texture_handles.len() {
                    material = material.with_emissive_texture(texture_handles[tex_index]);
                }
            }

            let emissive = gltf_mat.emissive_factor();
            let emissive_strength = (emissive[0] + emissive[1] + emissive[2]) / 3.0;
            if emissive_strength > 0.0 {
                material = material.with_emissive(emissive_strength);
            }

            // Occlusion
            if let Some(occlusion) = gltf_mat.occlusion_texture() {
                let tex_index = occlusion.texture().index();
                if tex_index < texture_handles.len() {
                    material = material.with_occlusion_texture(texture_handles[tex_index]);
                }
            }

            // Alpha mode
            if gltf_mat.alpha_mode() == gltf::material::AlphaMode::Blend {
                material = material.with_alpha();
            }

            log::debug!(
                "  Material '{}': metallic={:.2}, roughness={:.2}",
                mat_name,
                pbr.metallic_factor(),
                pbr.roughness_factor()
            );

            materials.push(material);
        }

        // Add a default material if none exist
        if materials.is_empty() {
            materials.push(Material::pbr());
        }

        Ok(materials)
    }

    /// Generate tangents for a mesh using a simplified MikkTSpace-like algorithm
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

        // Get indices as u32 iterator
        let index_iter: Vec<u32> = if let Some(idx) = indices {
            idx.clone().into_u32().collect()
        } else {
            (0..vertex_count as u32).collect()
        };

        // Process each triangle
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
                Vec3::X // Fallback
            };

            let bitangent = if f.is_finite() {
                Vec3::new(
                    f * (-delta_uv2.x * edge1.x + delta_uv1.x * edge2.x),
                    f * (-delta_uv2.x * edge1.y + delta_uv1.x * edge2.y),
                    f * (-delta_uv2.x * edge1.z + delta_uv1.x * edge2.z),
                )
            } else {
                Vec3::Y // Fallback
            };

            // Accumulate for averaging
            tangents[i0] += tangent;
            tangents[i1] += tangent;
            tangents[i2] += tangent;

            bitangents[i0] += bitangent;
            bitangents[i1] += bitangent;
            bitangents[i2] += bitangent;
        }

        // Orthonormalize and compute handedness
        tangents
            .iter()
            .zip(bitangents.iter())
            .zip(normals.iter())
            .map(|((t, b), n)| {
                let normal = Vec3::from(*n);
                let mut tangent = *t;

                // Gram-Schmidt orthogonalize
                tangent = (tangent - normal * normal.dot(tangent)).normalize_or_zero();

                // If tangent is zero (degenerate), create arbitrary tangent
                if tangent.length_squared() < 0.0001 {
                    tangent = if normal.y.abs() < 0.999 {
                        Vec3::Y.cross(normal).normalize()
                    } else {
                        Vec3::X.cross(normal).normalize()
                    };
                }

                // Calculate handedness
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

    fn load_primitive(
        primitive: &gltf::Primitive,
        buffers: &[gltf::buffer::Data],
        scene: &mut Scene,
        renderer: &mut Renderer,
        scale_multiplier: f32,
    ) -> Result<Handle<Mesh>, String> {
        let reader = primitive.reader(|buffer| Some(&buffers[buffer.index()]));

        // Read vertex data
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

        // Read tangents if available
        let tangents = reader
            .read_tangents()
            .map(|t| t.collect::<Vec<_>>())
            .unwrap_or_else(|| {
                log::debug!("    No tangents in glTF, generating them");
                // Generate tangents using MikkTSpace-like algorithm
                Self::generate_tangents(&positions, &normals, &uvs, &reader.read_indices())
            });

        // Read indices
        let indices = reader
            .read_indices()
            .ok_or("Missing indices")?
            .into_u32()
            .collect::<Vec<_>>();

        log::trace!(
            "    Primitive: {} vertices, {} indices",
            positions.len(),
            indices.len()
        );

        // Build vertices with tangents
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

        // Create mesh and store in assets
        let mesh = renderer.create_mesh(&vertices, &indices);
        let handle = scene.assets.meshes.insert(mesh);

        Ok(handle)
    }
}

#[cfg(target_arch = "wasm32")]
impl SceneLoader {
    fn import_gltf_web(
        path: &Path,
    ) -> Result<
        (
            gltf::Document,
            Vec<gltf::buffer::Data>,
            Vec<gltf::image::Data>,
        ),
        String,
    > {
        use gltf::Gltf;

        let bytes = crate::io::load_binary(path)?;
        let mut gltf = Gltf::from_slice(&bytes).map_err(|err| err.to_string())?;
        let document = gltf.document;
        let mut blob = gltf.blob;
        let base_dir = path.parent().map(|p| p.to_path_buf());

        let buffers = Self::import_buffers_web(&document, base_dir.as_deref(), &mut blob, path)?;
        let images = Self::import_images_web(&document, base_dir.as_deref(), &buffers)?;

        Ok((document, buffers, images))
    }

    fn import_buffers_web(
        document: &gltf::Document,
        base: Option<&Path>,
        blob: &mut Option<Vec<u8>>,
        original_path: &Path,
    ) -> Result<Vec<gltf::buffer::Data>, String> {
        let mut buffers = Vec::new();

        for buffer in document.buffers() {
            let mut data = match buffer.source() {
                gltf::buffer::Source::Uri(uri) => {
                    Self::load_external_resource(base, uri, Some(original_path))?
                }
                gltf::buffer::Source::Bin => blob
                    .take()
                    .ok_or_else(|| format!("Missing BIN chunk for buffer {}", buffer.index()))?,
            };

            while data.len() % 4 != 0 {
                data.push(0);
            }

            let expected = buffer.length() as usize;
            if data.len() < expected {
                return Err(format!(
                    "Buffer {} has {} bytes but expected {}",
                    buffer.index(),
                    data.len(),
                    expected
                ));
            }

            buffers.push(gltf::buffer::Data(data));
        }

        Ok(buffers)
    }

    fn import_images_web(
        document: &gltf::Document,
        base: Option<&Path>,
        buffers: &[gltf::buffer::Data],
    ) -> Result<Vec<gltf::image::Data>, String> {
        let mut images = Vec::new();

        for image in document.images() {
            let data = match image.source() {
                gltf::image::Source::Uri { uri, .. } => {
                    let bytes = Self::load_external_resource(base, uri, None)?;
                    Self::decode_image(&bytes)?
                }
                gltf::image::Source::View { view, .. } => {
                    let parent = &buffers[view.buffer().index()].0;
                    let begin = view.offset();
                    let end = begin + view.length();
                    if end > parent.len() {
                        return Err(format!(
                            "Image view for image {} is out of bounds",
                            image.index()
                        ));
                    }
                    Self::decode_image(&parent[begin..end])?
                }
            };

            images.push(data);
        }

        Ok(images)
    }

    fn decode_image(bytes: &[u8]) -> Result<gltf::image::Data, String> {
        use image::GenericImageView;

        let image = image::load_from_memory(bytes)
            .map_err(|err| format!("Failed to decode image data: {}", err))?;

        let format = match &image {
            image::DynamicImage::ImageLuma8(_) => gltf::image::Format::R8,
            image::DynamicImage::ImageLumaA8(_) => gltf::image::Format::R8G8,
            image::DynamicImage::ImageRgb8(_) => gltf::image::Format::R8G8B8,
            image::DynamicImage::ImageRgba8(_) => gltf::image::Format::R8G8B8A8,
            image::DynamicImage::ImageLuma16(_) => gltf::image::Format::R16,
            image::DynamicImage::ImageLumaA16(_) => gltf::image::Format::R16G16,
            image::DynamicImage::ImageRgb16(_) => gltf::image::Format::R16G16B16,
            image::DynamicImage::ImageRgba16(_) => gltf::image::Format::R16G16B16A16,
            image::DynamicImage::ImageRgb32F(_) => gltf::image::Format::R32G32B32FLOAT,
            image::DynamicImage::ImageRgba32F(_) => gltf::image::Format::R32G32B32A32FLOAT,
            other => return Err(format!("Unsupported image format: {:?}", other.color())),
        };

        let (width, height) = image.dimensions();
        let pixels = image.into_bytes();

        Ok(gltf::image::Data {
            pixels,
            format,
            width,
            height,
        })
    }

    fn load_external_resource(
        base: Option<&Path>,
        uri: &str,
        original_path: Option<&Path>,
    ) -> Result<Vec<u8>, String> {
        if let Some(rest) = uri.strip_prefix("data:") {
            let (_, encoded) = rest
                .split_once(",")
                .ok_or_else(|| format!("Malformed data URI: {}", uri))?;
            return base64::decode(encoded)
                .map_err(|err| format!("Failed to decode data URI: {}", err));
        }

        if uri.starts_with("http://") || uri.starts_with("https://") {
            return crate::io::load_binary_from_str(uri);
        }

        let path = if uri.starts_with('/') {
            std::path::PathBuf::from(uri.trim_start_matches('/'))
        } else if let Some(base_path) = base {
            base_path.join(uri)
        } else if let Some(orig) = original_path {
            orig.parent()
                .map(|p| p.join(uri))
                .ok_or_else(|| format!("Cannot resolve URI {}", uri))?
        } else {
            return Err(format!("Cannot resolve URI {}", uri));
        };

        crate::io::load_binary(&path)
    }
}
