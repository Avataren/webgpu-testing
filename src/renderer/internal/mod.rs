//! Internal building blocks that power the renderer.
//!
//! The renderer used to keep most of these helpers directly inside
//! `renderer.rs`, but the refactor that introduced this module surfaced the
//! distinct roles for batching, resource management, pipeline creation, and
//! shadow rendering.  These modules stay crate-private so the public renderer
//! API remains compact while still allowing the rest of the renderer crate to
//! share implementation details.

pub mod batches;
pub mod buffers;
pub mod context;
pub mod environment;
pub mod pipeline;
pub mod shadows;

pub(crate) use batches::{OrderedBatch, PreparedBatches};
pub(crate) use buffers::{CameraBuffer, DynamicObjectsBuffer, LightsBuffer};
pub(crate) use context::RenderContext;
pub(crate) use environment::EnvironmentResources;
pub(crate) use pipeline::{PipelineKey, RenderPipeline, TextureBindingModel};
pub(crate) use shadows::ShadowResources;
