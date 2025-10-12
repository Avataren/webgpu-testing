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
    particle_count: u32,
}

pub struct PhysicsBehavior {
    pub drag: f32,
    pub turbulence_strength: f32,
    pub turbulence_frequency: f32,
    pub gravity: Vec3,
}

impl Default for PhysicsBehavior {
    fn default() -> Self {
        Self {
            drag: 0.1,
            turbulence_strength: 0.0,
            turbulence_frequency: 1.0,
            gravity: Vec3::new(0.0, -9.81, 0.0),
        }
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
            particle_count: 0,
        };

        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("PhysicsParams"),
            contents: bytemuck::bytes_of(&params),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        })
    }

    fn update_params(&self, queue: &wgpu::Queue, buffer: &wgpu::Buffer, dt: f32) {
        let params = PhysicsParams {
            delta_time: dt,
            drag: self.drag,
            turbulence_strength: self.turbulence_strength,
            turbulence_frequency: self.turbulence_frequency,
            gravity: self.gravity.into(),
            particle_count: 0,
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
        assert_eq!(std::mem::size_of::<PhysicsParams>(), 32);
    }
}
