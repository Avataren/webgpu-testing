//! Hardware ray tracing through wgpu's experimental ray queries (Vulkan, DX12, Metal;
//! never on the web).
//!
//! Everything here is optional: the device only asks for ray queries when the adapter
//! has them, and every effect built on them falls back to its raster version when they
//! are missing or fail to set up. `WGPU_CUBE_DISABLE_RAY_TRACING=1` turns them off, to
//! test the fallbacks on hardware that has them.

mod scene;
mod shadows;

pub(crate) use scene::{RayTracingScene, RtCamera, MAX_CAUSTIC_CASTERS};
pub(crate) use shadows::{RtShadowStage, MASK_FORMAT as RT_SHADOW_MASK_FORMAT};

use std::fmt;

/// Device features the ray-traced effects need.
pub(crate) const RAY_TRACING_FEATURES: wgpu::Features = wgpu::Features::EXPERIMENTAL_RAY_QUERY;

/// Whether hardware ray tracing can be used on this device, and why not.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RayTracingSupport {
    Available,
    Unavailable(String),
}

impl RayTracingSupport {
    /// What the adapter offers, before the device is created.
    pub(crate) fn probe(adapter: &wgpu::Adapter) -> Self {
        if cfg!(target_arch = "wasm32") {
            return Self::Unavailable("not supported on the web".into());
        }
        let disabled = std::env::var("WGPU_CUBE_DISABLE_RAY_TRACING")
            .is_ok_and(|value| !matches!(value.as_str(), "" | "0" | "false"));
        if disabled {
            return Self::Unavailable("disabled by WGPU_CUBE_DISABLE_RAY_TRACING".into());
        }
        if !adapter.features().contains(RAY_TRACING_FEATURES) {
            return Self::Unavailable(format!(
                "adapter has no ray queries ({:?})",
                adapter.get_info().backend
            ));
        }
        let limits = adapter.limits();
        if limits.max_tlas_instance_count == 0
            || limits.max_acceleration_structures_per_shader_stage == 0
        {
            return Self::Unavailable("adapter reports no acceleration structure limits".into());
        }
        Self::Available
    }

    pub fn is_available(&self) -> bool {
        matches!(self, Self::Available)
    }
}

impl fmt::Display for RayTracingSupport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Available => f.write_str("available"),
            Self::Unavailable(reason) => write!(f, "unavailable: {reason}"),
        }
    }
}

/// Per-frame ray tracing resources a post-processing pass binds.
pub struct RayTracingFrame<'a> {
    /// The main pass's lights, shadow maps and environment (group 2).
    pub lights: &'a wgpu::BindGroup,
    /// The scene's acceleration structure and geometry (group 3).
    pub scene: &'a wgpu::BindGroup,
    /// The bindless texture array (group 4), on adapters that have one.
    pub textures: Option<&'a wgpu::BindGroup>,
    /// Whether the frame draws transmissive surfaces to refract through.
    pub refraction: bool,
}

/// How a particle system's particles combine with what is behind them (as they are drawn).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RayTracedParticleBlend {
    /// Solid: rays stop at them, as at any surface.
    Opaque,
    /// Over what is behind, by their alpha.
    AlphaBlend,
    /// Added to what is behind.
    Additive,
}

/// A GPU particle system, for rays to see (reflections and refraction): each frame its
/// live particles are expanded into triangles of `mesh` placed as the particle shader
/// places them, into a bottom-level structure of its own. Hand the renderer every system
/// that draws this frame with [`crate::renderer::Renderer::set_ray_traced_particles`].
#[derive(Clone, Debug)]
pub struct RayTracedParticles {
    /// Stays the same for the system's lifetime (its structure is kept by it).
    pub id: u64,
    /// The particles, laid out as `Particle` in particle_render.wgsl.
    pub particles: wgpu::Buffer,
    /// Particle slots the buffer holds.
    pub capacity: u32,
    /// Slots that may hold a live particle this frame (the rest are skipped).
    pub count: u32,
    pub mesh: crate::asset::Handle<crate::asset::Mesh>,
    pub material: crate::renderer::Material,
    pub blend: RayTracedParticleBlend,
}

/// Ray tracing counters for the last frame.
#[derive(Clone, Copy, Debug, Default)]
pub struct RayTracingStats {
    /// Instances in the top-level acceleration structure.
    pub instances: u32,
    /// Meshes with a bottom-level acceleration structure.
    pub meshes: u32,
    /// Bottom-level structures built this frame (new or changed meshes).
    pub blas_builds: u32,
    /// Vertices in the shared geometry pool (including space freed meshes left).
    pub pool_vertices: u32,
    /// Particle systems in the acceleration structure, and their live particle slots.
    pub particle_systems: u32,
    pub particles: u32,
}
