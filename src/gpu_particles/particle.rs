// src/gpu_particles/particle.rs
use bytemuck::{Pod, Zeroable};

pub const MAX_COLOR_KEYS: usize = 4;

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
    pub spawn_color: [f32; 4],
    pub color_keys: [[f32; 4]; MAX_COLOR_KEYS],
    pub color_key_times: [f32; MAX_COLOR_KEYS],
    /// User data layout:
    /// - `[0]`: spawn_scale - Size multiplier at spawn (for size curve interpolation)
    /// - `[1]`: size_ratio - Ratio between end size and start size
    /// - `[2]`: color_key_count - Number of valid color keys stored in the particle
    /// - `[3]`: reserved
    pub user_data: [f32; 4],
}

impl Default for Particle {
    fn default() -> Self {
        Self {
            position: [0.0; 3],
            lifetime: 0.0,
            velocity: [0.0; 3],
            max_lifetime: 5.0,
            rotation: Self::AXIS_ANGLE_IDENTITY,
            scale: [1.0; 3],
            angular_velocity: 0.0,
            color: [1.0; 4],
            spawn_color: [1.0; 4],
            color_keys: [[1.0; 4]; MAX_COLOR_KEYS],
            color_key_times: [0.0, 1.0, 1.0, 1.0],
            user_data: [1.0, 1.0, 1.0, 0.0], // Default: no size change, single color key
        }
    }
}

impl Particle {
    /// Axis-angle representation of the identity rotation used by the GPU shaders.
    pub const AXIS_ANGLE_IDENTITY: [f32; 4] = [0.0, 1.0, 0.0, 0.0];
    pub const MAX_COLOR_KEYS: usize = MAX_COLOR_KEYS;

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
