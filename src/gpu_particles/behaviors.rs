// src/gpu_particles/behaviors.rs

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

use super::ParticleBehavior;

// ============================================================================
// Starfield Behavior
// ============================================================================

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct StarfieldParams {
    delta_time: f32,
    near_plane: f32,
    far_plane: f32,
    far_reset_band: f32,
    field_half_size: f32,
    min_radius: f32,
    particle_count: u32,
    _padding: u32,
}

pub struct StarfieldBehavior {
    pub near_plane: f32,
    pub far_plane: f32,
    pub far_reset_band: f32,
    pub field_half_size: f32,
    pub min_radius: f32,
}

impl ParticleBehavior for StarfieldBehavior {
    fn shader_source(&self) -> &str {
        include_str!("shaders/starfield.wgsl")
    }

    fn entry_point(&self) -> &str {
        "main"
    }

    fn create_params_buffer(&self, device: &wgpu::Device, queue: &wgpu::Queue) -> wgpu::Buffer {
        let params = StarfieldParams {
            delta_time: 0.0,
            near_plane: self.near_plane,
            far_plane: self.far_plane,
            far_reset_band: self.far_reset_band,
            field_half_size: self.field_half_size,
            min_radius: self.min_radius,
            particle_count: 0,
            _padding: 0,
        };

        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("StarfieldParams"),
            contents: bytemuck::bytes_of(&params),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        })
    }

    fn update_params(&self, queue: &wgpu::Queue, buffer: &wgpu::Buffer, dt: f32) {
        let params = StarfieldParams {
            delta_time: dt,
            near_plane: self.near_plane,
            far_plane: self.far_plane,
            far_reset_band: self.far_reset_band,
            field_half_size: self.field_half_size,
            min_radius: self.min_radius,
            particle_count: 0,
            _padding: 0,
        };

        queue.write_buffer(buffer, 0, bytemuck::bytes_of(&params));
    }
}

// ============================================================================
// Physics Behavior
// ============================================================================

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
    pub gravity: glam::Vec3,
}

impl Default for PhysicsBehavior {
    fn default() -> Self {
        Self {
            drag: 0.1,
            turbulence_strength: 0.0,
            turbulence_frequency: 1.0,
            gravity: glam::Vec3::new(0.0, -9.81, 0.0),
        }
    }
}

impl ParticleBehavior for PhysicsBehavior {
    fn shader_source(&self) -> &str {
        include_str!("shaders/physics.wgsl")
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

// ============================================================================
// Boids Behavior
// ============================================================================

#[repr(C)]
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
        }
    }
}

impl ParticleBehavior for BoidsBehavior {
    fn shader_source(&self) -> &str {
        include_str!("shaders/boids.wgsl")
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
            particle_count: 0,
            _padding: 0,
        };

        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("BoidsParams"),
            contents: bytemuck::bytes_of(&params),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        })
    }

    fn update_params(&self, queue: &wgpu::Queue, buffer: &wgpu::Buffer, dt: f32) {
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
            particle_count: 0,
            _padding: 0,
        };

        queue.write_buffer(buffer, 0, bytemuck::bytes_of(&params));
    }
}