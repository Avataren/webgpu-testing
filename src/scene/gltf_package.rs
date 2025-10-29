use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Version of the packaged glTF descriptor format.
pub const PACKAGED_GLTF_VERSION: u32 = 2;

/// Describes the assets that comprise a packaged glTF import.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackagedGltfDescriptor {
    /// Descriptor schema version.
    pub version: u32,
    /// Legacy pointer to the original glTF asset. Present for backwards compatibility.
    #[serde(default)]
    pub source: Option<PathBuf>,
    /// Packaged scene payload information.
    #[serde(default)]
    pub scene: Option<PackagedScene>,
}

/// Metadata describing serialized scene state for a packaged glTF.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PackagedScene {
    /// Relative path to the serialized [`SceneAsset`] JSON file.
    pub json: PathBuf,
    /// Individual mesh payloads stored alongside the descriptor.
    #[serde(default)]
    pub meshes: Vec<PackagedMesh>,
}

/// A single mesh payload that belongs to a packaged glTF scene.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackagedMesh {
    /// Local mesh index within the scene asset.
    pub index: usize,
    /// Relative path to the serialized mesh data blob.
    pub path: PathBuf,
}
