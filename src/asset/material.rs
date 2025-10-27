use crate::renderer::Material;
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default)]
pub struct MaterialParameterMetadata {
    defaults: HashMap<String, Value>,
}

impl MaterialParameterMetadata {
    pub fn new() -> Self {
        Self {
            defaults: HashMap::new(),
        }
    }

    pub fn with_defaults(defaults: HashMap<String, Value>) -> Self {
        Self { defaults }
    }

    pub fn defaults(&self) -> &HashMap<String, Value> {
        &self.defaults
    }

    pub fn defaults_mut(&mut self) -> &mut HashMap<String, Value> {
        &mut self.defaults
    }

    pub fn insert_default(&mut self, name: impl Into<String>, value: Value) {
        self.defaults.insert(name.into(), value);
    }

    pub fn get(&self, name: &str) -> Option<&Value> {
        self.defaults.get(name)
    }

    pub fn is_empty(&self) -> bool {
        self.defaults.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AssetTypeTag(String);

impl AssetTypeTag {
    pub fn new(tag: impl Into<String>) -> Self {
        let tag = tag.into();
        if tag.is_empty() {
            Self("material".to_string())
        } else {
            Self(tag)
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for AssetTypeTag {
    fn default() -> Self {
        Self::new("material")
    }
}

#[derive(Debug, Clone)]
pub struct MaterialAsset {
    material: Material,
    canonical_path: PathBuf,
    default_parameters: MaterialParameterMetadata,
    asset_type: AssetTypeTag,
}

impl MaterialAsset {
    pub fn new(
        material: Material,
        canonical_path: PathBuf,
        default_parameters: MaterialParameterMetadata,
        asset_type: AssetTypeTag,
    ) -> Self {
        Self {
            material,
            canonical_path,
            default_parameters,
            asset_type,
        }
    }

    pub fn from_material(material: Material, canonical_path: PathBuf) -> Self {
        Self::new(
            material,
            canonical_path,
            MaterialParameterMetadata::default(),
            AssetTypeTag::default(),
        )
    }

    pub fn material(&self) -> &Material {
        &self.material
    }

    pub fn material_mut(&mut self) -> &mut Material {
        &mut self.material
    }

    pub fn canonical_path(&self) -> &Path {
        &self.canonical_path
    }

    pub fn set_canonical_path(&mut self, canonical_path: PathBuf) {
        self.canonical_path = canonical_path;
    }

    pub fn default_parameters(&self) -> &MaterialParameterMetadata {
        &self.default_parameters
    }

    pub fn default_parameters_mut(&mut self) -> &mut MaterialParameterMetadata {
        &mut self.default_parameters
    }

    pub fn asset_type(&self) -> &AssetTypeTag {
        &self.asset_type
    }

    pub fn set_asset_type(&mut self, asset_type: AssetTypeTag) {
        self.asset_type = asset_type;
    }
}

impl From<MaterialAsset> for Material {
    fn from(asset: MaterialAsset) -> Self {
        asset.material
    }
}
