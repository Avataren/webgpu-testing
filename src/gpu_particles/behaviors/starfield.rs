use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

use crate::gpu_particles::ParticleBehavior;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct StarfieldParams {
    time_and_planes: [f32; 4],
    field_and_count: [f32; 4],
}

pub struct StarfieldBehavior {
    pub near_plane: f32,
    pub far_plane: f32,
    pub far_reset_band: f32,
    pub field_half_size: f32,
    pub min_radius: f32,
}

impl ParticleBehavior for StarfieldBehavior {
    fn shader_source(&self) -> &'static str {
        include_str!("../shaders/starfield.wgsl")
    }

    fn entry_point(&self) -> &str {
        "main"
    }

    fn create_params_buffer(&self, device: &wgpu::Device, _queue: &wgpu::Queue) -> wgpu::Buffer {
        let params = StarfieldParams {
            time_and_planes: [0.0, self.near_plane, self.far_plane, self.far_reset_band],
            field_and_count: [self.field_half_size, self.min_radius, 0.0, 0.0],
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
            time_and_planes: [dt, self.near_plane, self.far_plane, self.far_reset_band],
            field_and_count: [
                self.field_half_size,
                self.min_radius,
                active_count as f32,
                0.0,
            ],
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

    #[test]
    fn starfield_params_layout() {
        use std::mem::offset_of;

        assert_eq!(offset_of!(StarfieldParams, time_and_planes), 0);
        assert_eq!(offset_of!(StarfieldParams, field_and_count), 16);
    }
}
