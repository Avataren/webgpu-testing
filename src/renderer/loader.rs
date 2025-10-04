// renderer/loader.rs
use crate::renderer::{Texture, Vertex};
use std::path::Path;

pub struct AssetLoader {
    device: wgpu::Device,
    queue: wgpu::Queue,
}

#[derive(Debug)]
pub enum LoadError {
    Io(std::io::Error),
    ImageError(String),
    GltfError(String),
    UnsupportedFormat,
}

impl From<std::io::Error> for LoadError {
    fn from(e: std::io::Error) -> Self {
        LoadError::Io(e)
    }
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoadError::Io(e) => write!(f, "IO error: {}", e),
            LoadError::ImageError(e) => write!(f, "Image error: {}", e),
            LoadError::GltfError(e) => write!(f, "GLTF error: {}", e),
            LoadError::UnsupportedFormat => write!(f, "Unsupported format"),
        }
    }
}

impl std::error::Error for LoadError {}

pub struct LoadedMesh {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u16>,
    pub name: Option<String>,
}

