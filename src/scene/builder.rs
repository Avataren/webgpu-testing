// scene/builder.rs
// Optional helper for building entities - uses pure hecs

use hecs::World;
use glam::Vec3;

use crate::asset::Handle;
use crate::asset::Mesh;
use crate::renderer::Material;
use crate::scene::Transform;
use super::components::*;

/// Helper for building entities with a fluent API
/// This is optional - you can also use world.spawn() directly
pub struct EntityBuilder<'w> {
    world: &'w mut World,
    builder: hecs::EntityBuilder,
}

impl<'w> EntityBuilder<'w> {
    /// Create a new entity builder
    pub fn new(world: &'w mut World) -> Self {
        Self {
            world,
            builder: hecs::EntityBuilder::new(),
        }
    }

    /// Add a name component
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.builder.add(Name::new(name));
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
    pub fn with_material(mut self, material: Material) -> Self {
        self.builder.add(MaterialComponent(material));
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
        self.world.spawn(self.builder.build())
    }
}

// Alternative: Direct usage without builder
// You can always use hecs directly like this:
/*
let entity = world.spawn((
    Name::new("Cube"),
    TransformComponent(Transform::default()),
    MeshComponent(mesh_handle),
    MaterialComponent(Material::white()),
    Visible(true),
));
*/