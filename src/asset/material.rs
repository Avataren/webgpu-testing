use crate::renderer::Material;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum MaterialTextureSlot {
    BaseColor,
    MetallicRoughness,
    Normal,
    Emissive,
    Occlusion,
}

impl MaterialTextureSlot {
    pub const fn all() -> [Self; 5] {
        [
            MaterialTextureSlot::BaseColor,
            MaterialTextureSlot::MetallicRoughness,
            MaterialTextureSlot::Normal,
            MaterialTextureSlot::Emissive,
            MaterialTextureSlot::Occlusion,
        ]
    }

    pub const fn expects_srgb(self) -> bool {
        matches!(
            self,
            MaterialTextureSlot::BaseColor | MaterialTextureSlot::Emissive
        )
    }
}

#[derive(Debug, Clone, Default)]
pub struct MaterialTextureReference {
    canonical_path: Option<PathBuf>,
    display_name: Option<String>,
}

impl MaterialTextureReference {
    pub fn new(canonical_path: Option<PathBuf>, display_name: Option<String>) -> Self {
        Self {
            canonical_path,
            display_name,
        }
    }

    pub fn with_path(path: PathBuf) -> Self {
        Self {
            canonical_path: Some(path),
            display_name: None,
        }
    }

    pub fn canonical_path(&self) -> Option<&Path> {
        self.canonical_path.as_deref()
    }

    pub fn set_canonical_path(&mut self, path: PathBuf) {
        self.canonical_path = Some(path);
    }

    pub fn clear_path(&mut self) {
        self.canonical_path = None;
    }

    pub fn display_name(&self) -> Option<&str> {
        self.display_name.as_deref()
    }

    pub fn set_display_name(&mut self, name: Option<String>) {
        self.display_name = name;
    }

    pub fn is_empty(&self) -> bool {
        self.canonical_path.is_none() && self.display_name.is_none()
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
    texture_references: BTreeMap<MaterialTextureSlot, MaterialTextureReference>,
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
            texture_references: BTreeMap::new(),
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

    pub fn texture_reference(
        &self,
        slot: MaterialTextureSlot,
    ) -> Option<&MaterialTextureReference> {
        self.texture_references.get(&slot)
    }

    pub fn texture_reference_mut(
        &mut self,
        slot: MaterialTextureSlot,
    ) -> &mut MaterialTextureReference {
        self.texture_references.entry(slot).or_default()
    }

    pub fn set_texture_reference(
        &mut self,
        slot: MaterialTextureSlot,
        reference: MaterialTextureReference,
    ) {
        if reference.is_empty() {
            self.texture_references.remove(&slot);
        } else {
            self.texture_references.insert(slot, reference);
        }
    }

    pub fn clear_texture_reference(&mut self, slot: MaterialTextureSlot) {
        self.texture_references.remove(&slot);
    }

    pub fn texture_references(
        &self,
    ) -> impl Iterator<Item = (MaterialTextureSlot, &MaterialTextureReference)> {
        self.texture_references
            .iter()
            .map(|(slot, reference)| (*slot, reference))
    }
}

impl From<MaterialAsset> for Material {
    fn from(asset: MaterialAsset) -> Self {
        asset.material
    }
}
