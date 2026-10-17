// src/renderer/mod.rs
pub mod batch;
pub mod compute_builder;
pub mod compute_pass;
pub mod compute_resources;
pub mod debug_draw;
pub mod depth;
pub mod frame_graph;
pub mod gizmo_shapes;
mod gpu_timer;
pub mod hdr_output;
pub(crate) mod internal;
pub(crate) mod light_culling;
pub mod light_probe_debug;
pub mod light_probes;
pub mod lights;
pub mod material;
pub mod objects;
pub mod pipeline_builder;
pub mod postprocess;
pub mod primitives;
pub mod raytracing;
pub mod readback;
pub mod render_context;
pub mod render_region;
mod renderer_core;
pub mod shader_builder;
pub mod shader_source;
pub mod skinning;
pub mod texture;
pub mod uniforms;
pub mod vertex;

pub const MAX_TEXTURES: usize = 256;

pub type PickId = u64;

pub use batch::{
    Batch, InstanceData, RenderBatcher, RenderObject, RenderPass, SkinBinding, SkinJob, SkinKey,
};
pub use compute_builder::ComputePipelineBuilder;
pub use compute_pass::{dispatch_compute, ComputePass};
pub use compute_resources::{
    BindGroupBuilder, BindGroupLayoutBuilder, StorageBuffer, UniformBuffer,
};
pub use depth::Depth;
pub use gpu_timer::GpuTimings;
pub use light_probe_debug::LightProbeDebugView;
pub use light_probes::{
    LightProbeBakeOutcome, LightProbeBakeProgress, LightProbeGrid, MAX_LIGHT_PROBE_VOLUMES,
};
pub use lights::{
    DirectionalShadowData, LightsData, LocalLight, SpotCone, MAX_DIRECTIONAL_LIGHTS,
    MAX_LIGHTS_PER_CLUSTER, MAX_LOCAL_LIGHTS, MAX_SHADOWED_LOCAL_LIGHTS, MAX_SHADOW_CASCADES,
};
pub use material::Material;
pub use objects::{MaterialData, ObjectData};
pub use pipeline_builder::PipelineBuilder;
pub use primitives::*;
pub use render_context::{
    CustomRenderCallback, CustomRenderContext, CustomRenderRequest, CustomRenderStage,
    ShadowPassStage,
};
pub use render_region::RenderRegion;
pub use renderer_core::{
    LightShadowState, LightStats, RenderFrame, Renderer, RendererStats, ShaderReload, SurfaceError,
    TAA_JITTER_FRAMES,
};
pub use shader_builder::{SamplerFilterMode, ShaderBuilder};
pub use texture::Texture;
pub use uniforms::CameraUniform;
pub use vertex::{SkinVertex, Vertex};
