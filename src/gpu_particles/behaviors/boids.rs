use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

use crate::gpu_particles::ParticleBehavior;

#[repr(C, align(16))]
#[derive(Clone, Copy, Pod, Zeroable)]
struct BoidsParams {
    delta_time: f32,
    separation_radius: f32,
    alignment_radius: f32,
    cohesion_radius: f32,
    separation_weight: f32,
    alignment_weight: f32,
    cohesion_weight: f32,
    max_speed: f32,
    max_force: f32,
    bounds: f32,
    particle_count: u32,
    _padding: u32,
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
    fn shader_source(&self) -> &str {
        include_str!("../shaders/boids.wgsl")
    }

    fn entry_point(&self) -> &str {
        "main"
    }

    fn create_params_buffer(&self, device: &wgpu::Device, _queue: &wgpu::Queue) -> wgpu::Buffer {
        let params = BoidsParams {
            delta_time: 0.0,
            separation_radius: self.separation_radius,
            alignment_radius: self.alignment_radius,
            cohesion_radius: self.cohesion_radius,
            separation_weight: self.separation_weight,
            alignment_weight: self.alignment_weight,
            cohesion_weight: self.cohesion_weight,
            max_speed: self.max_speed,
            max_force: self.max_force,
            bounds: self.bounds,
            particle_count: self.particle_count,
            _padding: 0,
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
    ) {
        let params = BoidsParams {
            delta_time: dt,
            separation_radius: self.separation_radius,
            alignment_radius: self.alignment_radius,
            cohesion_radius: self.cohesion_radius,
            separation_weight: self.separation_weight,
            alignment_weight: self.alignment_weight,
            cohesion_weight: self.cohesion_weight,
            max_speed: self.max_speed,
            max_force: self.max_force,
            bounds: self.bounds,
            particle_count: active_count,
            _padding: 0,
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
        assert_eq!(std::mem::align_of::<BoidsParams>(), 16);
    }
}
