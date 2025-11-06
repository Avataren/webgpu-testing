mod serde_helpers;
mod animation;
mod components;
mod material;
mod transform;
mod script;

// Re-export all public types
pub use serde_helpers::ImportedGltfMeta;
pub(crate) use serde_helpers::{
    flatten_gltf_material_key, material_key_map_serde, material_map_serde, path_serde,
};

pub use animation::{
    SerializedAnimationChannel, SerializedAnimationClip, SerializedAnimationOutput,
    SerializedAnimationSampler, SerializedAnimationTarget,
};

pub use components::{
    SerializedBillboard, SerializedBillboardOrientation, SerializedBillboardSpace,
    SerializedDirectionalLight, SerializedMeshBounds, SerializedParticleBehavior,
    SerializedParticleEmitter, SerializedParticleSystem, SerializedPointLight,
    SerializedSpotLight,
};

pub use material::{
    SerializedMaterial, SerializedMaterialKind, SerializedTextureSlot,
};

pub use transform::SerializedTransform;

pub use script::{SerializedRuneScript, SerializedRuneScriptSource};
