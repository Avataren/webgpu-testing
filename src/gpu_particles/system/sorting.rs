use std::sync::{Arc, Mutex};

use bytemuck::cast_slice;
use glam::Vec3;

use crate::gpu_particles::particle::Particle;

pub(crate) struct ParticleSorting {
    readback_buffer: wgpu::Buffer,
    staging: Vec<Particle>,
    sorted_indices: Vec<u32>,
    pending_readback: bool,
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
        let readback_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ParticlesReadback"),
            size: buffer_size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        Self {
            readback_buffer,
            staging: Vec::with_capacity(max_particles as usize),
            sorted_indices: Vec::with_capacity(max_particles as usize),
            pending_readback: false,
            last_camera_position: Vec3::ZERO,
            needs_alpha_sorting,
        }
    }

    pub(crate) fn needs_alpha_sorting(&self) -> bool {
        self.needs_alpha_sorting
    }

    pub(crate) fn try_complete_readback(&mut self, device: &wgpu::Device, render_high_water: u32) {
        if !self.needs_alpha_sorting || !self.pending_readback {
            return;
        }

        if render_high_water == 0 {
            self.pending_readback = false;
            return;
        }

        let readback_size = (render_high_water as usize * std::mem::size_of::<Particle>()) as u64;
        let slice = self.readback_buffer.slice(..readback_size);
        let status = Arc::new(Mutex::new(None));
        let status_clone = Arc::clone(&status);

        slice.map_async(wgpu::MapMode::Read, move |result| {
            *status_clone.lock().unwrap() = Some(result);
        });

        let _ = device.poll(wgpu::PollType::wait_indefinitely());

        let mut guard = status.lock().unwrap();
        if let Some(result) = guard.take() {
            drop(guard);
            match result {
                Ok(()) => {
                    let data = slice.get_mapped_range();
                    let particles = cast_slice::<u8, Particle>(&data);

                    let available = particles.len().min(render_high_water as usize);
                    self.staging.clear();
                    self.staging.extend_from_slice(&particles[..available]);

                    drop(data);
                    self.readback_buffer.unmap();

                    self.sort_particles(available as u32);
                    self.pending_readback = false;
                }
                Err(error) => {
                    log::warn!("Failed to map particles for sorting: {:?}", error);
                    self.pending_readback = false;
                }
            }
        }
    }

    pub(crate) fn schedule_readback(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        particles_buffer: &wgpu::Buffer,
        render_high_water: u32,
    ) {
        if !self.needs_alpha_sorting || self.pending_readback || render_high_water == 0 {
            return;
        }

        let copy_size = (render_high_water as usize * std::mem::size_of::<Particle>()) as u64;
        encoder.copy_buffer_to_buffer(particles_buffer, 0, &self.readback_buffer, 0, copy_size);
        self.pending_readback = true;
    }

    pub(crate) fn sorted_indices(&self) -> &[u32] {
        &self.sorted_indices
    }

    pub(crate) fn record_camera_position(&mut self, position: Vec3) {
        self.last_camera_position = position;
    }

    pub(crate) fn reset(&mut self) {
        self.pending_readback = false;
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
