// scene/loader.rs
// glTF loader using pure hecs

use std::path::Path;
use glam::{Quat, Vec3};

use crate::asset::Handle;
use crate::asset::Mesh;
use crate::renderer::{Material, Renderer, Vertex};
use crate::scene::{Scene, Transform};
use super::components::*;

pub struct SceneLoader;

impl SceneLoader {
    /// Load a glTF file into the scene
    pub fn load_gltf(
        path: impl AsRef<Path>,
        scene: &mut Scene,
        renderer: &mut Renderer,
    ) -> Result<(), String> {
        let (document, buffers, _images) = gltf::import(path)
            .map_err(|e| format!("Failed to load glTF: {}", e))?;

        log::info!("Loading glTF: {} meshes", document.meshes().len());

        // Load all meshes
        let mut mesh_handles = Vec::new();
        for gltf_mesh in document.meshes() {
            for primitive in gltf_mesh.primitives() {
                let handle = Self::load_primitive(&primitive, &buffers, scene, renderer)?;
                mesh_handles.push(handle);
            }
        }

        // Load all nodes (creates entities)
        for gltf_scene in document.scenes() {
            log::info!("Loading scene: {:?}", gltf_scene.name());
            for node in gltf_scene.nodes() {
                Self::load_node(&node, None, &mesh_handles, &mut scene.world)?;
            }
        }

        log::info!("glTF loaded: {} entities created", scene.world.len());
        Ok(())
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

    /// Load a glTF node recursively
    fn load_node(
        node: &gltf::Node,
        parent: Option<hecs::Entity>,
        mesh_handles: &[Handle<Mesh>],
        world: &mut hecs::World,
    ) -> Result<hecs::Entity, String> {
        // Get transform from glTF
        let (translation, rotation, scale) = node.transform().decomposed();
        let transform = Transform {
            translation: Vec3::from(translation),
            rotation: Quat::from_array(rotation),
            scale: Vec3::from(scale),
        };

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
            if let Some(&mesh_handle) = mesh_handles.get(gltf_mesh.index()) {
                entity_builder.add(MeshComponent(mesh_handle));
                entity_builder.add(MaterialComponent(Material::white()));
            }
        }

        // Spawn the entity
        let entity = world.spawn(entity_builder.build());

        // Load children recursively
        let mut children = Vec::new();
        for child in node.children() {
            let child_entity = Self::load_node(&child, Some(entity), mesh_handles, world)?;
            children.push(child_entity);
        }

        // Add children component if we have children
        if !children.is_empty() {
            world.insert_one(entity, Children(children)).ok();
        }

        Ok(entity)
    }
}