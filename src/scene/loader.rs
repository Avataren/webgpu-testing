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

        // Apply scale multiplier to convert units
        transform.scale *= scale_multiplier;
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
            log::debug!("  Has mesh with {} primitives", gltf_mesh.primitives().len());
            
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
            log::debug!("  Adding {} children to entity {:?}", children.len(), entity);
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
                let handle = Self::load_primitive(&primitive, &buffers, scene, renderer)?;
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
                log::info!("    Loading root node {}/{}: {:?}", 
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
                        true, // sRGB
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

    /// Load a mesh primitive
    fn load_primitive(
        primitive: &gltf::Primitive,
        buffers: &[gltf::buffer::Data],
        scene: &mut Scene,
        renderer: &mut Renderer,
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

        // Read indices
        let indices = reader
            .read_indices()
            .ok_or("Missing indices")?
            .into_u32()
            .map(|i| i as u16)
            .collect::<Vec<_>>();

        log::trace!(
            "    Primitive: {} vertices, {} indices",
            positions.len(),
            indices.len()
        );

        // Build vertices
        let vertices = positions
            .iter()
            .zip(normals.iter())
            .zip(uvs.iter())
            .map(|((pos, norm), uv)| Vertex {
                pos: *pos,
                normal: *norm,
                uv: *uv,
            })
            .collect::<Vec<_>>();

        // Create mesh and store in assets
        let mesh = renderer.create_mesh(&vertices, &indices);
        let handle = scene.assets.meshes.insert(mesh);

        Ok(handle)
    }
}