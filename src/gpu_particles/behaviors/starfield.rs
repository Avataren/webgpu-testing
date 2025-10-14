use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

use crate::gpu_particles::ParticleBehavior;

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
        include_str!("../shaders/starfield.wgsl")
    }

    fn entry_point(&self) -> &str {
        "main"
    }

    fn create_params_buffer(&self, device: &wgpu::Device, _queue: &wgpu::Queue) -> wgpu::Buffer {
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

    fn update_params(
        &self,
        queue: &wgpu::Queue,
        buffer: &wgpu::Buffer,
        dt: f32,
        active_count: u32,
    ) {
        let params = StarfieldParams {
            delta_time: dt,
            near_plane: self.near_plane,
            far_plane: self.far_plane,
            far_reset_band: self.far_reset_band,
            field_half_size: self.field_half_size,
            min_radius: self.min_radius,
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
    fn starfield_params_alignment() {
        assert_eq!(std::mem::size_of::<StarfieldParams>(), 32);
    }
}
