use bytemuck::{Pod, Zeroable};
use glam::Mat4;
use wgpu::util::DeviceExt;

use crate::gpu_particles::ParticleBehavior;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct BoidsParams {
    radii: [f32; 4],
    weights_and_speed: [f32; 4],
    force_bounds_and_count: [f32; 4],
}

pub struct BoidsBehavior {
    pub separation_radius: f32,
    pub alignment_radius: f32,
    pub cohesion_radius: f32,
    pub separation_weight: f32,
    pub alignment_weight: f32,
    pub cohesion_weight: f32,
    pub max_speed: f32,
    pub max_force: f32,
    pub bounds: f32,
    pub particle_count: u32,
}

impl Default for BoidsBehavior {
    fn default() -> Self {
        Self {
            separation_radius: 2.0,
            alignment_radius: 4.0,
            cohesion_radius: 4.0,
            separation_weight: 1.5,
            alignment_weight: 1.0,
            cohesion_weight: 1.0,
            max_speed: 5.0,
            max_force: 0.5,
            bounds: 20.0,
            particle_count: 0,
        }
    }
}

impl BoidsBehavior {
    pub fn set_particle_count(&mut self, count: u32) {
        self.particle_count = count;
    }
}

impl ParticleBehavior for BoidsBehavior {
    fn shader_source(&self) -> &'static str {
        include_str!("../shaders/boids.wgsl")
    }

    fn entry_point(&self) -> &str {
        "main"
    }

    fn create_params_buffer(&self, device: &wgpu::Device, _queue: &wgpu::Queue) -> wgpu::Buffer {
        let params = BoidsParams {
            radii: [
                0.0,
                self.separation_radius,
                self.alignment_radius,
                self.cohesion_radius,
            ],
            weights_and_speed: [
                self.separation_weight,
                self.alignment_weight,
                self.cohesion_weight,
                self.max_speed,
            ],
            force_bounds_and_count: [self.max_force, self.bounds, self.particle_count as f32, 0.0],
        };

        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("BoidsParams"),
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
        _emitter_transform: Mat4,
    ) {
        let params = BoidsParams {
            radii: [
                dt,
                self.separation_radius,
                self.alignment_radius,
                self.cohesion_radius,
            ],
            weights_and_speed: [
                self.separation_weight,
                self.alignment_weight,
                self.cohesion_weight,
                self.max_speed,
            ],
            force_bounds_and_count: [self.max_force, self.bounds, active_count as f32, 0.0],
        };

        queue.write_buffer(buffer, 0, bytemuck::bytes_of(&params));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boids_default_values() {
        let boids = BoidsBehavior::default();
        assert_eq!(boids.separation_radius, 2.0);
        assert_eq!(boids.alignment_radius, 4.0);
        assert_eq!(boids.cohesion_radius, 4.0);
        assert_eq!(boids.separation_weight, 1.5);
        assert_eq!(boids.alignment_weight, 1.0);
        assert_eq!(boids.cohesion_weight, 1.0);
        assert_eq!(boids.max_speed, 5.0);
        assert_eq!(boids.max_force, 0.5);
        assert_eq!(boids.bounds, 20.0);
        assert_eq!(boids.particle_count, 0);
    }

    #[test]
    fn boids_set_particle_count() {
        let mut boids = BoidsBehavior::default();
        boids.set_particle_count(1000);
        assert_eq!(boids.particle_count, 1000);
    }

    #[test]
    fn boids_params_alignment() {
        assert_eq!(std::mem::size_of::<BoidsParams>(), 48);
    }

    #[test]
    fn boids_params_layout() {
        use std::mem::offset_of;

        assert_eq!(offset_of!(BoidsParams, radii), 0);
        assert_eq!(offset_of!(BoidsParams, weights_and_speed), 16);
        assert_eq!(offset_of!(BoidsParams, force_bounds_and_count), 32);
    }
}
