// src/gpu_particles/particle.rs
use bytemuck::{Pod, Zeroable};
use glam::Quat;

#[repr(C, align(16))]
#[derive(Clone, Copy, Pod, Zeroable, Debug)]
pub struct Particle {
    pub position: [f32; 3],
    pub lifetime: f32,
    pub velocity: [f32; 3],
    pub max_lifetime: f32,
    pub rotation: [f32; 4],
    pub scale: [f32; 3],
    pub angular_velocity: f32,
    pub color: [f32; 4],
    pub user_data: [f32; 4], // [start_size, end_size, unused, unused]
}

impl Default for Particle {
    fn default() -> Self {
        Self {
            position: [0.0; 3],
            lifetime: 0.0,
            velocity: [0.0; 3],
            max_lifetime: 5.0,
            rotation: Quat::IDENTITY.into(),
            scale: [1.0; 3],
            angular_velocity: 0.0,
            color: [1.0; 4],
            user_data: [1.0, 1.0, 0.0, 0.0], // Default: no size change
        }
    }
}

impl Particle {
    /// Check if this particle is dead (available for recycling)
    pub fn is_dead(&self) -> bool {
        self.lifetime < 0.0 || self.lifetime >= self.max_lifetime
    }

    /// Mark this particle as dead/available for recycling
    pub fn mark_dead(&mut self) {
        self.lifetime = -1.0;
        self.position = [0.0, -10000.0, 0.0]; // Hide far offscreen
    }
}
