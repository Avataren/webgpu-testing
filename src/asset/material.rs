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

/// Describes the high level kind of a material asset.
#[derive(Debug, Clone)]
pub enum MaterialKind {
    /// A built-in physically based rendering (PBR) material.
    Pbr,
    /// A user-authored shader material with custom WGSL source code.
    Shader(ShaderMaterialMetadata),
}

impl Default for MaterialKind {
    fn default() -> Self {
        Self::Pbr
    }
}

/// Metadata for a shader material, including its WGSL source and compatibility flags.
#[derive(Debug, Clone)]
pub struct ShaderMaterialMetadata {
    wgsl_source: String,
    needs_lighting_include: bool,
    source_path: Option<PathBuf>,
}

impl ShaderMaterialMetadata {
    /// Default WGSL template used for newly created shader materials.
    ///
    /// The template documents the include markers that the renderer understands:
    /// `// @include_material_system`, `// @include_lighting`,
    /// `// @include_shadows`, and `// @include_environment`.
    pub const DEFAULT_TEMPLATE: &'static str =
        include_str!("../shader/templates/shader_material.wgsl");

    /// Creates metadata with explicit WGSL source.
    pub fn new(wgsl_source: impl Into<String>) -> Self {
        Self {
            wgsl_source: wgsl_source.into(),
            needs_lighting_include: false,
            source_path: None,
        }
    }

    /// Creates metadata populated with the default shader template.
    pub fn default_template() -> Self {
        Self {
            wgsl_source: Self::DEFAULT_TEMPLATE.to_string(),
            // The default template opts into shared lighting via marker comments.
            // Retain the legacy flag for compatibility with serialized assets.
            needs_lighting_include: true,
            source_path: None,
        }
    }

    /// Returns the WGSL source for the shader material.
    pub fn wgsl_source(&self) -> &str {
        &self.wgsl_source
    }

    /// Provides mutable access to the WGSL source string.
    pub fn wgsl_source_mut(&mut self) -> &mut String {
        &mut self.wgsl_source
    }

    /// Replaces the WGSL source with a new value.
    pub fn set_wgsl_source(&mut self, source: impl Into<String>) {
        self.wgsl_source = source.into();
    }

    /// Returns the file system path backing this shader material, if any.
    pub fn source_path(&self) -> Option<&Path> {
        self.source_path.as_deref()
    }

    /// Sets or clears the shader source path tracked by this metadata.
    pub fn set_source_path(&mut self, path: Option<PathBuf>) {
        self.source_path = path;
    }

    /// Indicates whether the shader should explicitly include the shared lighting module.
    ///
    /// New shader materials should prefer the `// @include_*` markers documented in the
    /// template, but the flag is kept for compatibility with previously serialized assets.
    pub fn needs_lighting_include(&self) -> bool {
        self.needs_lighting_include
    }

    /// Sets whether the shader requires the shared lighting include file.
    pub fn set_needs_lighting_include(&mut self, needs_include: bool) {
        self.needs_lighting_include = needs_include;
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
    kind: MaterialKind,
}

impl MaterialAsset {
    pub fn new(
        material: Material,
        canonical_path: PathBuf,
        default_parameters: MaterialParameterMetadata,
        asset_type: AssetTypeTag,
        kind: MaterialKind,
    ) -> Self {
        Self {
            material,
            canonical_path,
            default_parameters,
            asset_type,
            texture_references: BTreeMap::new(),
            kind,
        }
    }

    pub fn from_material(material: Material, canonical_path: PathBuf) -> Self {
        Self::new(
            material,
            canonical_path,
            MaterialParameterMetadata::default(),
            AssetTypeTag::default(),
            MaterialKind::Pbr,
        )
    }

    /// Creates a shader material asset with the default WGSL template.
    ///
    /// The template includes documentation for the supported `// @include_*` markers,
    /// allowing users to opt into shared lighting, shadows, and environment helpers.
    pub fn shader(material: Material, canonical_path: PathBuf) -> Self {
        Self::new(
            material,
            canonical_path,
            MaterialParameterMetadata::default(),
            AssetTypeTag::new("shader_material"),
            MaterialKind::Shader(ShaderMaterialMetadata::default_template()),
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

    pub fn kind(&self) -> &MaterialKind {
        &self.kind
    }

    pub fn kind_mut(&mut self) -> &mut MaterialKind {
        &mut self.kind
    }

    pub fn set_kind(&mut self, kind: MaterialKind) {
        self.kind = kind;
    }

    pub fn shader_metadata(&self) -> Option<&ShaderMaterialMetadata> {
        match &self.kind {
            MaterialKind::Shader(metadata) => Some(metadata),
            MaterialKind::Pbr => None,
        }
    }

    pub fn shader_metadata_mut(&mut self) -> Option<&mut ShaderMaterialMetadata> {
        match &mut self.kind {
            MaterialKind::Shader(metadata) => Some(metadata),
            MaterialKind::Pbr => None,
        }
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
