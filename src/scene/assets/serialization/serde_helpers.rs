use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub mod path_serde {
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

pub mod material_map_serde {
    use serde::{Deserialize, Deserializer, Serializer};
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    pub fn serialize<S>(value: &BTreeMap<usize, PathBuf>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mapped: BTreeMap<String, String> = value
            .iter()
            .map(|(index, path)| (index.to_string(), path.to_string_lossy().into_owned()))
            .collect();
        serde::Serialize::serialize(&mapped, serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<BTreeMap<usize, PathBuf>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let mapped = BTreeMap::<String, String>::deserialize(deserializer)?;
        let mut result = BTreeMap::new();
        for (key, value) in mapped {
            let index = key.parse::<usize>().map_err(serde::de::Error::custom)?;
            result.insert(index, PathBuf::from(value));
        }
        Ok(result)
    }
}

pub mod material_key_map_serde {
    use serde::{Deserialize, Deserializer, Serializer};
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    pub fn serialize<S>(value: &BTreeMap<String, PathBuf>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mapped: BTreeMap<&str, String> = value
            .iter()
            .map(|(key, path)| (key.as_str(), path.to_string_lossy().into_owned()))
            .collect();
        serde::Serialize::serialize(&mapped, serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<BTreeMap<String, PathBuf>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let mapped = BTreeMap::<String, String>::deserialize(deserializer)?;
        Ok(mapped
            .into_iter()
            .map(|(key, value)| (key, PathBuf::from(value)))
            .collect())
    }
}

pub(crate) fn flatten_gltf_material_key(
    node: Option<usize>,
    primitive: Option<usize>,
    material_index: Option<usize>,
) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();

    if let Some(node) = node {
        parts.push(format!("node:{node}"));
    }

    if let Some(primitive) = primitive {
        parts.push(format!("primitive:{primitive}"));
    }

    if let Some(material_index) = material_index {
        parts.push(format!("material:{material_index}"));
    }

    if parts.is_empty() {
        None
    } else {
        Some(parts.join("/"))
    }
}

/// Metadata recorded for a glTF import to preserve material assignments across reloads.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ImportedGltfMeta {
    /// Map of glTF material indices to relative material asset paths.
    #[serde(default, with = "material_map_serde")]
    pub materials: BTreeMap<usize, PathBuf>,
    /// Map of glTF material bindings keyed by a flattened [`GltfMaterialKey`].
    #[serde(default, with = "material_key_map_serde")]
    pub materials_by_key: BTreeMap<String, PathBuf>,
}

impl ImportedGltfMeta {
    pub fn record_material(&mut self, index: usize, path: PathBuf) {
        self.materials.insert(index, path);
    }

    pub fn record_material_key(
        &mut self,
        key: &crate::scene::gltf_material_registry::GltfMaterialKey,
        path: PathBuf,
    ) {
        if let Some(flattened) =
            flatten_gltf_material_key(key.node, key.primitive, key.material_index)
        {
            self.materials_by_key.insert(flattened, path);
        }
    }

    pub fn lookup_material_path(
        &self,
        node: Option<usize>,
        primitive: Option<usize>,
        material_index: Option<usize>,
    ) -> Option<&PathBuf> {
        if let Some(material_index) = material_index {
            if let Some(key) = flatten_gltf_material_key(node, primitive, Some(material_index)) {
                if let Some(path) = self.materials_by_key.get(&key) {
                    return Some(path);
                }
            }
        }

        if let Some(key) = flatten_gltf_material_key(node, primitive, None) {
            if let Some(path) = self.materials_by_key.get(&key) {
                return Some(path);
            }
        }

        if let Some(material_index) = material_index {
            if let Some(path) = self.materials.get(&material_index) {
                return Some(path);
            }

            if let Some(key) = flatten_gltf_material_key(None, None, Some(material_index)) {
                if let Some(path) = self.materials_by_key.get(&key) {
                    return Some(path);
                }
            }
        }

        None
    }
}
