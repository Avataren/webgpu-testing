// scene/builder.rs
// Optional helper for building entities - uses pure hecs

use super::components::*;
use super::Scene;
use crate::asset::{Handle, MaterialAsset, Mesh};
use crate::scene::Transform;
use crate::scripting::{LuaScriptComponent, LuaScriptSource};
use crate::{asset::Assets, renderer::Material};
use glam::Vec3;
use std::path::PathBuf;

#[derive(Debug)]
pub enum MaterialAssignment {
    Handle(Handle<MaterialAsset>),
    Asset(MaterialAsset),
    Material {
        material: Material,
        canonical_path: Option<PathBuf>,
    },
}

pub trait IntoMaterialAssignment {
    fn into_material_assignment(self) -> MaterialAssignment;
}

impl IntoMaterialAssignment for Handle<MaterialAsset> {
    fn into_material_assignment(self) -> MaterialAssignment {
        MaterialAssignment::Handle(self)
    }
}

impl IntoMaterialAssignment for MaterialAsset {
    fn into_material_assignment(self) -> MaterialAssignment {
        MaterialAssignment::Asset(self)
    }
}

impl IntoMaterialAssignment for Material {
    fn into_material_assignment(self) -> MaterialAssignment {
        MaterialAssignment::Material {
            material: self,
            canonical_path: None,
        }
    }
}

impl<P> IntoMaterialAssignment for (P, Material)
where
    P: Into<PathBuf>,
{
    fn into_material_assignment(self) -> MaterialAssignment {
        MaterialAssignment::Material {
            material: self.1,
            canonical_path: Some(self.0.into()),
        }
    }
}

/// Helper for building entities with a fluent API
/// This is optional - you can also use world.spawn() directly
pub struct EntityBuilder<'w> {
    scene: &'w mut Scene,
    builder: hecs::EntityBuilder,
}

impl<'w> EntityBuilder<'w> {
    /// Create a new entity builder
    pub fn new(scene: &'w mut Scene) -> Self {
        Self {
            scene,
            builder: hecs::EntityBuilder::new(),
        }
    }

    /// Add a name component
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.builder.add(Name::new(name));
        self
    }

    /// Add any custom component (generic method)
    pub fn with_component<T: hecs::Component>(mut self, component: T) -> Self {
        self.builder.add(component);
        self
    }

    /// Add a transform component
    pub fn with_transform(mut self, transform: Transform) -> Self {
        self.builder.add(TransformComponent(transform));
        self
    }

    /// Add a mesh component
    pub fn with_mesh(mut self, mesh: Handle<Mesh>) -> Self {
        self.builder.add(MeshComponent(mesh));
        self
    }

    /// Add a material component
    pub fn with_material<M>(mut self, material: M) -> Self
    where
        M: IntoMaterialAssignment,
    {
        let handle = match material.into_material_assignment() {
            MaterialAssignment::Handle(handle) => handle,
            MaterialAssignment::Asset(asset) => self.insert_material_asset(asset),
            MaterialAssignment::Material {
                material,
                canonical_path,
            } => {
                let asset =
                    MaterialAsset::from_material(material, canonical_path.unwrap_or_default());
                self.insert_material_asset(asset)
            }
        };

        self.builder.add(MaterialComponent(handle));
        self
    }

    /// Add a visibility component
    pub fn visible(mut self, visible: bool) -> Self {
        self.builder.add(Visible(visible));
        self
    }

    /// Add a rotation animation component
    pub fn with_rotation_animation(mut self, axis: Vec3, speed: f32) -> Self {
        self.builder.add(RotateAnimation { axis, speed });
        self
    }

    /// Add a particle system component
    pub fn with_particle_system(mut self, component: ParticleSystemComponent) -> Self {
        self.builder.add(component);
        self
    }

    /// Add a particle emitter component
    pub fn with_particle_emitter(mut self, component: ParticleEmitterComponent) -> Self {
        self.builder.add(component);
        self
    }

    /// Attach a scripting component to the entity.
    pub fn with_script(mut self, source: LuaScriptSource) -> Self {
        self.builder.add(LuaScriptComponent::new(source));
        self
    }

    /// Add an orbit animation component
    pub fn with_orbit_animation(
        mut self,
        center: Vec3,
        radius: f32,
        speed: f32,
        offset: f32,
    ) -> Self {
        self.builder.add(OrbitAnimation {
            center,
            radius,
            speed,
            offset,
        });
        self
    }

    /// Spawn the entity into the world
    pub fn spawn(&mut self) -> hecs::Entity {
        self.scene.main_world_mut().spawn(self.builder.build())
    }

    fn insert_material_asset(&mut self, asset: MaterialAsset) -> Handle<MaterialAsset> {
        Self::insert_material_asset_inner(&mut self.scene.assets, asset)
    }

    fn insert_material_asset_inner(
        assets: &mut Assets,
        asset: MaterialAsset,
    ) -> Handle<MaterialAsset> {
        assets.insert_material_asset(asset)
    }
}

// Alternative: Direct usage without builder
// You can always use hecs directly like this:
/*
let entity = world.spawn((
    Name::new("Cube"),
    TransformComponent(Transform::default()),
    MeshComponent(mesh_handle),
    MaterialComponent(material_handle),
    Visible(true),
));
*/
