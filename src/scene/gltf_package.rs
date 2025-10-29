mod path_serde {
    use serde::{Deserialize, Deserializer, Serializer};
    use std::path::{Path, PathBuf};

    pub fn serialize<S>(path: &Path, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(path.to_string_lossy().as_ref())
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<PathBuf, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Ok(PathBuf::from(value))
    }
}

use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

const DESCRIPTOR_FORMAT: &str = "wgpu-cube/packaged-gltf";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackagedGltfDependency {
    #[serde(with = "path_serde")]
    pub original: PathBuf,
    #[serde(with = "path_serde")]
    pub packaged: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackagedGltfDescriptor {
    pub format: String,
    #[serde(with = "path_serde")]
    pub source: PathBuf,
    #[serde(default)]
    pub dependencies: Vec<PackagedGltfDependency>,
}

impl PackagedGltfDescriptor {
    pub fn new(source: PathBuf) -> Self {
        Self {
            format: DESCRIPTOR_FORMAT.to_string(),
            source,
            dependencies: Vec::new(),
        }
    }

    pub fn add_dependency(&mut self, original: PathBuf, packaged: PathBuf) {
        self.dependencies
            .push(PackagedGltfDependency { original, packaged });
    }

    pub fn save_to_path(&self, path: &Path) -> io::Result<()> {
        let json = serde_json::to_string_pretty(self).map_err(io::Error::other)?;
        fs::write(path, json)
    }

    pub fn load_from_path(path: &Path) -> io::Result<Option<Self>> {
        match fs::read_to_string(path) {
            Ok(contents) => match serde_json::from_str::<Self>(&contents) {
                Ok(descriptor) if descriptor.format == DESCRIPTOR_FORMAT => Ok(Some(descriptor)),
                Ok(_) => Ok(None),
                Err(_) => Ok(None),
            },
            Err(err) => {
                if err.kind() == io::ErrorKind::InvalidData
                    || err.kind() == io::ErrorKind::UnexpectedEof
                {
                    Ok(None)
                } else {
                    Err(err)
                }
            }
        }
    }
}
