use bytemuck::cast_slice;
use glam::Vec3;

use super::readback::AsyncReadback;
use crate::gpu_particles::particle::Particle;

pub(crate) struct ParticleSorting {
    readback: AsyncReadback,
    staging: Vec<Particle>,
    sorted_indices: Vec<u32>,
    last_camera_position: Vec3,
    needs_alpha_sorting: bool,
}

impl ParticleSorting {
    pub(crate) fn new(
        device: &wgpu::Device,
        max_particles: u32,
        needs_alpha_sorting: bool,
    ) -> Self {
        let buffer_size = (max_particles as usize * std::mem::size_of::<Particle>()) as u64;

        Self {
            readback: AsyncReadback::new(device, buffer_size, "ParticlesReadback"),
            staging: Vec::with_capacity(max_particles as usize),
            sorted_indices: Vec::with_capacity(max_particles as usize),
            last_camera_position: Vec3::ZERO,
            needs_alpha_sorting,
        }
    }

    pub(crate) fn needs_alpha_sorting(&self) -> bool {
        self.needs_alpha_sorting
    }

    /// Takes the particles copied by an earlier `schedule_readback` once they have arrived
    /// and sorts them back to front. The order is a few frames old, which is fine for sorting.
    pub(crate) fn try_complete_readback(&mut self, device: &wgpu::Device, render_high_water: u32) {
        if !self.needs_alpha_sorting {
            return;
        }

        if render_high_water == 0 {
            self.readback.reset();
            return;
        }

        let staging = &mut self.staging;
        let arrived = self.readback.poll(device, |bytes| {
            let particles = cast_slice::<u8, Particle>(bytes);
            staging.clear();
            staging.extend_from_slice(particles);
        });
        if arrived {
            self.sort_particles(self.staging.len() as u32);
        }
    }

    pub(crate) fn schedule_readback(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        particles_buffer: &wgpu::Buffer,
        render_high_water: u32,
    ) {
        if !self.needs_alpha_sorting || render_high_water == 0 {
            return;
        }

        let copy_size = (render_high_water as usize * std::mem::size_of::<Particle>()) as u64;
        self.readback.schedule(encoder, particles_buffer, copy_size);
    }

    pub(crate) fn sorted_indices(&self) -> &[u32] {
        &self.sorted_indices
    }

    pub(crate) fn record_camera_position(&mut self, position: Vec3) {
        self.last_camera_position = position;
    }

    pub(crate) fn reset(&mut self) {
        self.readback.reset();
        self.sorted_indices.clear();
        self.staging.clear();
    }

    fn sort_particles(&mut self, count: u32) {
        self.sorted_indices.clear();

        for index in 0..count {
            let particle = &self.staging[index as usize];
            if particle.lifetime >= 0.0 && particle.lifetime < particle.max_lifetime {
                self.sorted_indices.push(index);
            }
        }

        self.sorted_indices.sort_by(|&a, &b| {
            let pos_a = Vec3::from(self.staging[a as usize].position);
            let pos_b = Vec3::from(self.staging[b as usize].position);

            let dist_a = self.last_camera_position.distance_squared(pos_a);
            let dist_b = self.last_camera_position.distance_squared(pos_b);

            dist_b
                .partial_cmp(&dist_a)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
    }
}
