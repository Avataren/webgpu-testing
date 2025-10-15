// src/gpu_particles/system/mod.rs
mod gpu_particle_system;
mod pipeline;
mod shadow;
mod slot_allocator;
mod slot_allocator_v2;
mod sorting;

pub use gpu_particle_system::{GpuParticleSystem, ParticleRenderMode};

// Add this type alias for easy migration
#[cfg(not(feature = "optimized-allocator"))]
pub(crate) type ActiveSlotAllocator = slot_allocator::ParticleSlotAllocator;

#[cfg(feature = "optimized-allocator")]
pub(crate) type ActiveSlotAllocator = slot_allocator_v2::SlotAllocator;
