// src/gpu_particles/behaviors/physics.rs
use bytemuck::{Pod, Zeroable};
use glam::Vec3;
use wgpu::util::DeviceExt;

use crate::gpu_particles::ParticleBehavior;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct PhysicsParams {
    delta_time: f32,
    drag: f32,
    turbulence_strength: f32,
    turbulence_frequency: f32,
    gravity: [f32; 3],
    _padding_vec3: f32, // ✅ vec3 needs padding to 16 bytes
    particle_count: u32,
    ground_level: f32,
    bounce_factor: f32,
    velocity_damping: f32,
}

pub struct PhysicsBehavior {
    pub drag: f32,
    pub turbulence_strength: f32,
    pub turbulence_frequency: f32,
    pub gravity: Vec3,
    pub ground_level: f32,
    pub bounce_factor: f32,
    pub velocity_damping: f32,
}

impl Default for PhysicsBehavior {
    fn default() -> Self {
        Self {
            drag: 0.1,
            turbulence_strength: 0.0,
            turbulence_frequency: 1.0,
            gravity: Vec3::new(0.0, -9.81, 0.0),
            ground_level: 0.0,
            bounce_factor: 0.3,
            velocity_damping: 0.8,
        }
    }
}

impl PhysicsBehavior {
    pub fn with_gravity(mut self, gravity: Vec3) -> Self {
        self.gravity = gravity;
        self
    }

    pub fn with_drag(mut self, drag: f32) -> Self {
        self.drag = drag;
        self
    }

    pub fn with_ground_collision(mut self, ground_level: f32, bounce: f32, damping: f32) -> Self {
        self.ground_level = ground_level;
        self.bounce_factor = bounce;
        self.velocity_damping = damping;
        self
    }

    pub fn with_turbulence(mut self, strength: f32, frequency: f32) -> Self {
        self.turbulence_strength = strength;
        self.turbulence_frequency = frequency;
        self
    }
}

impl ParticleBehavior for PhysicsBehavior {
    fn shader_source(&self) -> &str {
        include_str!("../shaders/physics.wgsl")
    }

    fn entry_point(&self) -> &str {
        "main"
    }

    fn create_params_buffer(&self, device: &wgpu::Device, _queue: &wgpu::Queue) -> wgpu::Buffer {
        let params = PhysicsParams {
            delta_time: 0.0,
            drag: self.drag,
            turbulence_strength: self.turbulence_strength,
            turbulence_frequency: self.turbulence_frequency,
            gravity: self.gravity.into(),
            _padding_vec3: 0.0, // ✅ Initialize padding
            particle_count: 0,
            ground_level: self.ground_level,
            bounce_factor: self.bounce_factor,
            velocity_damping: self.velocity_damping,
        };

        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("PhysicsParams"),
            contents: bytemuck::bytes_of(&params),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        })
    }

    fn update_params(
        &self,
        queue: &wgpu::Queue,
        buffer: &wgpu::Buffer,
        dt: f32,
        active_count: u32,
    ) {
        let params = PhysicsParams {
            delta_time: dt,
            drag: self.drag,
            turbulence_strength: self.turbulence_strength,
            turbulence_frequency: self.turbulence_frequency,
            gravity: self.gravity.into(),
            _padding_vec3: 0.0, // ✅ Initialize padding
            particle_count: active_count,
            ground_level: self.ground_level,
            bounce_factor: self.bounce_factor,
            velocity_damping: self.velocity_damping,
        };

        queue.write_buffer(buffer, 0, bytemuck::bytes_of(&params));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn physics_default_values() {
        let physics = PhysicsBehavior::default();
        assert_eq!(physics.drag, 0.1);
        assert_eq!(physics.turbulence_strength, 0.0);
        assert_eq!(physics.turbulence_frequency, 1.0);
        assert_eq!(physics.gravity, Vec3::new(0.0, -9.81, 0.0));
    }

    #[test]
    fn physics_params_alignment() {
        // Size should still be 48 bytes, but now properly aligned
        assert_eq!(std::mem::size_of::<PhysicsParams>(), 48);
    }

    #[test]
    fn physics_params_layout() {
        use std::mem::offset_of;

        // Verify proper vec3 alignment
        assert_eq!(offset_of!(PhysicsParams, gravity), 16);
        assert_eq!(offset_of!(PhysicsParams, particle_count), 32); // After 16-byte aligned vec3
        assert_eq!(offset_of!(PhysicsParams, ground_level), 36);
        assert_eq!(offset_of!(PhysicsParams, bounce_factor), 40);
        assert_eq!(offset_of!(PhysicsParams, velocity_damping), 44);
    }
}
