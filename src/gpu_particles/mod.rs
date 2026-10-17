// src/gpu_particles/mod.rs
mod behavior;
mod emitter;
mod particle;
mod shader_modules;
pub(crate) mod system;

pub mod behaviors;
pub mod pools;
pub mod runtime;

pub use behavior::ParticleBehavior;
pub use emitter::{ColorGradient, EmissionShape, ParticleEmitter, SizeCurve};
pub use particle::Particle;
pub use runtime::{ParticleBurst, ParticleRuntime};
pub use system::{GpuParticleSystem, ParticleRenderMode};
