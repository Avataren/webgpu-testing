use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Version of the packaged glTF descriptor format.
pub const PACKAGED_GLTF_VERSION: u32 = 3;

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
    /// Individual texture payloads stored alongside the descriptor.
    #[serde(default)]
    pub textures: Vec<PackagedTexture>,
}

/// A single mesh payload that belongs to a packaged glTF scene.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackagedMesh {
    /// Local mesh index within the scene asset.
    pub index: usize,
    /// Relative path to the serialized mesh data blob.
    pub path: PathBuf,
}

/// A single texture payload that belongs to a packaged glTF scene.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackagedTexture {
    /// Local texture index within the scene asset.
    pub index: u32,
    /// Relative path to the encoded texture image on disk.
    pub path: PathBuf,
}
