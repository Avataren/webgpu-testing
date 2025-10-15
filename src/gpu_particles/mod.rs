// src/gpu_particles/mod.rs
mod behavior;
mod emitter;
mod particle;
mod shader_modules;
mod system;

pub mod behaviors;

pub use behavior::ParticleBehavior;
pub use emitter::{ColorGradient, EmissionShape, ParticleEmitter, SizeCurve};
pub use particle::Particle;
pub use system::GpuParticleSystem;
