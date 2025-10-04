// scene/scene.rs
// Pure hecs ECS scene implementation

use hecs::World;
use glam::{Quat, Vec3};

use crate::asset::Assets;
use crate::renderer::{RenderBatcher, RenderObject, Renderer};
use super::components::*;

pub struct Scene {
    /// hecs World - stores all entities and components
    pub world: World,
    
    /// Asset storage (meshes, textures)
    pub assets: Assets,
    
    /// Total elapsed time
    time: f64,
    
    /// Last frame timestamp
    last_frame: std::time::Instant,
}

impl Scene {
    pub fn new() -> Self {
        Self {
            world: World::new(),
            assets: Assets::default(),
            time: 0.0,
            last_frame: std::time::Instant::now(),
        }
    }

    // ========================================================================
    // Time Management
    // ========================================================================

    pub fn time(&self) -> f64 {
        self.time
    }

    pub fn last_frame(&self) -> std::time::Instant {
        self.last_frame
    }

    pub fn set_last_frame(&mut self, instant: std::time::Instant) {
        self.last_frame = instant;
    }

    // ========================================================================
    // Update - runs all systems
    // ========================================================================

    pub fn update(&mut self, dt: f64) {
        self.time += dt;
        
        // Run systems
        self.system_rotate_animation(dt);
        self.system_orbit_animation(dt);
    }

    // ========================================================================
    // Rendering
    // ========================================================================

    pub fn render(&mut self, renderer: &mut Renderer, batcher: &mut RenderBatcher) {
        batcher.clear();

        // Query all renderable entities
        // This is pure hecs - query for components we need
        for (_entity, (transform, mesh, material, visible)) in self
            .world
            .query::<(&TransformComponent, &MeshComponent, &MaterialComponent, &Visible)>()
            .iter()
        {
            if !visible.0 {
                continue;
            }

            batcher.add(RenderObject {
                mesh: mesh.0,
                material: material.0,
                transform: transform.0,
            });
        }

        if let Err(e) = renderer.render(&self.assets, batcher) {
            log::error!("Render error: {:?}", e);
        }
    }

    // ========================================================================
    // Systems - pure hecs queries
    // ========================================================================

    /// System: Update rotation animations
    fn system_rotate_animation(&mut self, dt: f64) {
        for (_entity, (transform, anim)) in self
            .world
            .query::<(&mut TransformComponent, &RotateAnimation)>()
            .iter()
        {
            let rotation = Quat::from_axis_angle(anim.axis, anim.speed * dt as f32);
            transform.0.rotation = rotation * transform.0.rotation;
        }
    }

    /// System: Update orbit animations
    fn system_orbit_animation(&mut self, dt: f64) {
        let time = self.time as f32;
        
        for (_entity, (transform, orbit)) in self
            .world
            .query::<(&mut TransformComponent, &OrbitAnimation)>()
            .iter()
        {
            let angle = time * orbit.speed + orbit.offset;
            transform.0.translation = orbit.center + Vec3::new(
                angle.cos() * orbit.radius,
                (time + orbit.offset).sin() * 0.5,
                angle.sin() * orbit.radius,
            );
        }
    }
}

impl Default for Scene {
    fn default() -> Self {
        Self::new()
    }
}