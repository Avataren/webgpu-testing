// scene/loader.rs (PBR version)
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
        mesh_handles: &[(Handle<Mesh>, Option<usize>)],
        materials: &[Material],
        world: &mut hecs::World,
        scale_multiplier: f32, // Add this parameter
    ) -> Result<hecs::Entity, String> {
        // Get transform from glTF
        let (translation, rotation, scale) = node.transform().decomposed();
        let mut transform = Transform {
            translation: Vec3::from(translation),
            rotation: Quat::from_array(rotation),
            scale: Vec3::from(scale),
        };

        // Apply scale multiplier (useful for models in different units)
        transform.scale *= scale_multiplier;
        transform.translation *= scale_multiplier;

        // Build entity using pure hecs
        let mut entity_builder = hecs::EntityBuilder::new();

        // Add name
        entity_builder.add(Name::new(node.name().unwrap_or("Unnamed")));

        // Add transform
        entity_builder.add(TransformComponent(transform));

        // Add visibility
        entity_builder.add(Visible(true));

        // Add parent if exists
        if let Some(parent_entity) = parent {
            entity_builder.add(Parent(parent_entity));
        }

        // Add mesh and material if this node has a mesh
        if let Some(gltf_mesh) = node.mesh() {
            if let Some(&(mesh_handle, material_index)) = mesh_handles.get(gltf_mesh.index()) {
                entity_builder.add(MeshComponent(mesh_handle));

                // Get the material for this primitive
                let material = if let Some(mat_idx) = material_index {
                    materials.get(mat_idx).copied().unwrap_or(Material::pbr())
                } else {
                    Material::pbr()
                };

                entity_builder.add(MaterialComponent(material));
            }
        }

        // Spawn the entity
        let entity = world.spawn(entity_builder.build());

        // Load children recursively (pass scale multiplier down)
        let mut children = Vec::new();
        for child in node.children() {
            let child_entity = Self::load_node(
                &child,
                Some(entity),
                mesh_handles,
                materials,
                world,
                scale_multiplier,
            )?;
            children.push(child_entity);
        }

        // Add children component if we have children
        if !children.is_empty() {
            world.insert_one(entity, Children(children)).ok();
        }

        Ok(entity)
    }

    /// Load a glTF file into the scene with scale
    pub fn load_gltf(
        path: impl AsRef<Path>,
        scene: &mut Scene,
        renderer: &mut Renderer,
        scale: f32, // Add scale parameter
    ) -> Result<(), String> {
        let path = path.as_ref();
        let (document, buffers, images) =
            gltf::import(path).map_err(|e| format!("Failed to load glTF: {}", e))?;

        log::info!(
            "Loading glTF: {} meshes, {} materials, {} textures",
            document.meshes().len(),
            document.materials().len(),
            document.textures().len()
        );

        // Get the base directory for loading external textures
        let base_dir = path.parent().unwrap_or_else(|| Path::new("."));

        // Load all textures first
        let texture_handles = Self::load_textures(&document, &images, base_dir, scene, renderer)?;

        // Load all materials
        let material_handles = Self::load_materials(&document, &texture_handles)?;

        // Load all meshes
        let mut mesh_handles = Vec::new();
        for gltf_mesh in document.meshes() {
            for primitive in gltf_mesh.primitives() {
                let handle = Self::load_primitive(&primitive, &buffers, scene, renderer)?;
                mesh_handles.push((handle, primitive.material().index()));
            }
        }

        // Load all nodes (creates entities)
        for gltf_scene in document.scenes() {
            log::info!(
                "Loading scene: {:?} with scale {}",
                gltf_scene.name(),
                scale
            );
            for node in gltf_scene.nodes() {
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

        log::info!("glTF loaded: {} entities created", scene.world.len());
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
                    // Load from file
                    let texture_path = base_dir.join(uri);
                    log::info!("Loading texture from file: {:?}", texture_path);

                    // Determine if sRGB based on usage (base color should be sRGB, others linear)
                    let is_srgb = true; // We'll handle this more precisely in the shader

                    Texture::from_path(
                        renderer.get_device(),
                        renderer.get_queue(),
                        &texture_path,
                        is_srgb,
                    )?
                }
                gltf::image::Source::View { .. } => {
                    // Load from embedded buffer
                    let img_data = &images[source.index()];
                    log::info!(
                        "Loading embedded texture: {}x{}",
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

            log::info!(
                "Loaded material: {:?}, metallic={:.2}, roughness={:.2}",
                gltf_mat.name(),
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
