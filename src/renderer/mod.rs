// src/renderer/mod.rs
pub mod batch;
pub mod compute_builder;
pub mod compute_pass;
pub mod compute_resources;
pub mod depth;
pub(crate) mod internal;
pub mod lights;
pub mod material;
pub mod objects;
pub mod pipeline_builder;
pub mod postprocess;
pub mod primitives;
pub mod render_context;
pub mod render_region;
mod renderer_core;
pub mod shader_builder;
pub mod texture;
pub mod uniforms;
pub mod vertex;

pub const MAX_TEXTURES: usize = 256;

pub use batch::{Batch, InstanceData, RenderBatcher, RenderObject, RenderPass};
pub use compute_builder::ComputePipelineBuilder;
pub use compute_pass::{dispatch_compute, ComputePass};
pub use compute_resources::{
    BindGroupBuilder, BindGroupLayoutBuilder, StorageBuffer, UniformBuffer,
};
pub use depth::Depth;
pub use lights::{
    DirectionalShadowData, LightsData, PointShadowData, SpotLightDescriptor, SpotShadowData,
    MAX_DIRECTIONAL_LIGHTS, MAX_POINT_LIGHTS, MAX_SPOT_LIGHTS,
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
pub use renderer_core::{RenderFrame, Renderer, RendererStats};
pub use shader_builder::{SamplerFilterMode, ShaderBuilder};
pub use texture::Texture;
pub use uniforms::CameraUniform;
pub use vertex::Vertex;
