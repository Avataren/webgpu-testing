use crate::asset::Assets;
use crate::asset::{
    MaterialAsset, MaterialKind, MaterialTextureReference, MaterialTextureSlot,
    ShaderMaterialMetadata,
};
use crate::project::{
    active_project_root, normalize_absolute_path, relativize_path_to_project, resolve_project_path,
};
use crate::renderer::material::MaterialFlags;
use crate::renderer::texture::{
    DEFAULT_METALLIC_ROUGHNESS_TEXTURE_INDEX, DEFAULT_NORMAL_TEXTURE_INDEX,
    DEFAULT_WHITE_TEXTURE_INDEX,
};
use crate::renderer::Material;
use crate::scene::loader::SceneImportDevice;
use serde::de::{self, MapAccess, Visitor};
use serde::ser::SerializeMap;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Default)]
pub struct SerializedTextureSlot {
    pub path: Option<PathBuf>,
    pub name: Option<String>,
    pub index: Option<u32>,
}

impl SerializedTextureSlot {
    pub(crate) fn from_index(index: u32) -> Self {
        Self {
            path: None,
            name: None,
            index: Some(index),
        }
    }

    pub(crate) fn remap(&mut self, mapping: &std::collections::HashMap<u32, u32>) {
        if let Some(index) = self.index {
            if let Some(&mapped) = mapping.get(&index) {
                self.index = Some(mapped);
            }
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.path.is_none() && self.name.is_none() && self.index.is_none()
    }
}

impl Serialize for SerializedTextureSlot {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        if self.path.is_none() && self.name.is_none() {
            if let Some(index) = self.index {
                serializer.serialize_u32(index)
            } else {
                serializer.serialize_none()
            }
        } else {
            let mut entries = 0;
            if self.path.is_some() {
                entries += 1;
            }
            if self.name.is_some() {
                entries += 1;
            }
            if self.path.is_none() && self.index.is_some() {
                entries += 1;
            }

            let mut map = serializer.serialize_map(Some(entries))?;
            if let Some(path) = &self.path {
                map.serialize_entry("path", &path.to_string_lossy().to_string())?;
            }
            if let Some(name) = &self.name {
                map.serialize_entry("name", name)?;
            }
            if self.path.is_none() {
                if let Some(index) = self.index {
                    map.serialize_entry("index", &index)?;
                }
            }
            map.end()
        }
    }
}

impl<'de> Deserialize<'de> for SerializedTextureSlot {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct SlotVisitor;

        impl<'de> Visitor<'de> for SlotVisitor {
            type Value = SerializedTextureSlot;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("a texture index or metadata map")
            }

            fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(SerializedTextureSlot::from_index(value as u32))
            }

            fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                if value < 0 {
                    return Err(E::custom("negative texture index"));
                }
                Ok(SerializedTextureSlot::from_index(value as u32))
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(SerializedTextureSlot {
                    path: Some(PathBuf::from(value)),
                    name: None,
                    index: None,
                })
            }

            fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                self.visit_str(&value)
            }

            fn visit_none<E>(self) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(SerializedTextureSlot::default())
            }

            fn visit_unit<E>(self) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(SerializedTextureSlot::default())
            }

            fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
            where
                M: MapAccess<'de>,
            {
                let mut path: Option<PathBuf> = None;
                let mut name: Option<String> = None;
                let mut index: Option<u32> = None;

                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "path" => {
                            let value: String = map.next_value()?;
                            path = Some(PathBuf::from(value));
                        }
                        "name" => {
                            name = Some(map.next_value()?);
                        }
                        "index" => {
                            index = Some(map.next_value()?);
                        }
                        _ => {
                            let _: serde_json::Value = map.next_value()?;
                        }
                    }
                }

                Ok(SerializedTextureSlot { path, name, index })
            }
        }

        deserializer.deserialize_any(SlotVisitor)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializedMaterial {
    pub base_color: [u8; 4],
    pub flags: u32,
    #[serde(default, skip_serializing_if = "SerializedTextureSlot::is_empty")]
    pub base_color_texture: SerializedTextureSlot,
    #[serde(default, skip_serializing_if = "SerializedTextureSlot::is_empty")]
    pub metallic_roughness_texture: SerializedTextureSlot,
    #[serde(default, skip_serializing_if = "SerializedTextureSlot::is_empty")]
    pub normal_texture: SerializedTextureSlot,
    #[serde(default, skip_serializing_if = "SerializedTextureSlot::is_empty")]
    pub emissive_texture: SerializedTextureSlot,
    #[serde(default, skip_serializing_if = "SerializedTextureSlot::is_empty")]
    pub occlusion_texture: SerializedTextureSlot,
    pub metallic_factor: u8,
    pub roughness_factor: u8,
    pub emissive_strength: u8,
    #[serde(default)]
    pub kind: SerializedMaterialKind,
}

impl From<&MaterialAsset> for SerializedMaterial {
    fn from(asset: &MaterialAsset) -> Self {
        SerializedMaterial::from_material_asset(asset)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SerializedMaterialKind {
    Pbr,
    Shader {
        wgsl_source: String,
        #[serde(default)]
        needs_lighting_include: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        shader_path: Option<String>,
    },
}

impl Default for SerializedMaterialKind {
    fn default() -> Self {
        Self::Pbr
    }
}

impl SerializedMaterialKind {
    fn from_material_kind(kind: &MaterialKind) -> Self {
        match kind {
            MaterialKind::Pbr => Self::Pbr,
            MaterialKind::Shader(metadata) => Self::Shader {
                wgsl_source: metadata.wgsl_source().to_string(),
                needs_lighting_include: metadata.needs_lighting_include(),
                shader_path: metadata
                    .source_path()
                    .map(|path| path.to_string_lossy().to_string()),
            },
        }
    }

    fn to_material_kind(&self) -> MaterialKind {
        match self {
            SerializedMaterialKind::Pbr => MaterialKind::Pbr,
            SerializedMaterialKind::Shader {
                wgsl_source,
                needs_lighting_include,
                shader_path,
            } => {
                let mut metadata = ShaderMaterialMetadata::new(wgsl_source.clone());
                metadata.set_needs_lighting_include(*needs_lighting_include);
                if let Some(path) = shader_path {
                    metadata.set_source_path(Some(PathBuf::from(path)));
                }
                MaterialKind::Shader(metadata)
            }
        }
    }
}

impl SerializedMaterial {
    pub(crate) fn from_material(material: Material) -> Self {
        Self {
            base_color: material.base_color,
            flags: material.flags.bits(),
            base_color_texture: SerializedTextureSlot::from_index(material.base_color_texture),
            metallic_roughness_texture: SerializedTextureSlot::from_index(
                material.metallic_roughness_texture,
            ),
            normal_texture: SerializedTextureSlot::from_index(material.normal_texture),
            emissive_texture: SerializedTextureSlot::from_index(material.emissive_texture),
            occlusion_texture: SerializedTextureSlot::from_index(material.occlusion_texture),
            metallic_factor: material.metallic_factor,
            roughness_factor: material.roughness_factor,
            emissive_strength: material.emissive_strength,
            kind: SerializedMaterialKind::default(),
        }
    }

    pub(crate) fn remap_textures(&mut self, mapping: &std::collections::HashMap<u32, u32>) {
        self.base_color_texture.remap(mapping);
        self.metallic_roughness_texture.remap(mapping);
        self.normal_texture.remap(mapping);
        self.emissive_texture.remap(mapping);
        self.occlusion_texture.remap(mapping);
    }

    pub(crate) fn texture_slot(&self, slot: MaterialTextureSlot) -> &SerializedTextureSlot {
        match slot {
            MaterialTextureSlot::BaseColor => &self.base_color_texture,
            MaterialTextureSlot::MetallicRoughness => &self.metallic_roughness_texture,
            MaterialTextureSlot::Normal => &self.normal_texture,
            MaterialTextureSlot::Emissive => &self.emissive_texture,
            MaterialTextureSlot::Occlusion => &self.occlusion_texture,
        }
    }

    pub(crate) fn texture_slot_mut(
        &mut self,
        slot: MaterialTextureSlot,
    ) -> &mut SerializedTextureSlot {
        match slot {
            MaterialTextureSlot::BaseColor => &mut self.base_color_texture,
            MaterialTextureSlot::MetallicRoughness => &mut self.metallic_roughness_texture,
            MaterialTextureSlot::Normal => &mut self.normal_texture,
            MaterialTextureSlot::Emissive => &mut self.emissive_texture,
            MaterialTextureSlot::Occlusion => &mut self.occlusion_texture,
        }
    }

    pub fn from_material_asset(asset: &MaterialAsset) -> Self {
        let mut serialized = SerializedMaterial::from_material(*asset.material());
        serialized.kind = SerializedMaterialKind::from_material_kind(asset.kind());
        let project_root = active_project_root();
        let canonical_project_root = project_root
            .as_ref()
            .and_then(|root| std::fs::canonicalize(root).ok())
            .map(normalize_absolute_path);

        for slot in MaterialTextureSlot::all() {
            if let Some(reference) = asset.texture_reference(slot) {
                let slot_mut = serialized.texture_slot_mut(slot);
                if let Some(path) = reference.canonical_path() {
                    let mut stored_path = path.to_path_buf();
                    if let Some(root) = project_root.as_deref() {
                        stored_path = relativize_path_to_project(
                            stored_path,
                            root,
                            canonical_project_root.as_deref(),
                        );
                    }
                    slot_mut.path = Some(stored_path);
                }
                if let Some(name) = reference.display_name() {
                    slot_mut.name = Some(name.to_string());
                }
            }
        }

        serialized
    }

    pub fn resolve_material(
        &self,
        assets: &mut Assets,
        mut renderer: Option<&mut dyn SceneImportDevice>,
    ) -> (
        Material,
        MaterialKind,
        Vec<(MaterialTextureSlot, Option<MaterialTextureReference>)>,
    ) {
        let mut material = Material::from(self.clone());
        let kind = self.kind.to_material_kind();
        let mut references = Vec::new();

        for slot in MaterialTextureSlot::all() {
            let slot_data = self.texture_slot(slot);
            let mut resolved_path = slot_data.path.as_ref().map(resolve_project_path);

            let texture_index = if let Some(resolved) = resolved_path.as_deref() {
                super::super::resources::with_import_device(&mut renderer, |device| {
                    assets.resolve_texture_index(slot, Some(resolved), device, false)
                })
            } else if let Some(index) = slot_data.index {
                index
            } else {
                Assets::default_texture_index(slot)
            };

            match slot {
                MaterialTextureSlot::BaseColor => material.base_color_texture = texture_index,
                MaterialTextureSlot::MetallicRoughness => {
                    material.metallic_roughness_texture = texture_index
                }
                MaterialTextureSlot::Normal => material.normal_texture = texture_index,
                MaterialTextureSlot::Emissive => material.emissive_texture = texture_index,
                MaterialTextureSlot::Occlusion => material.occlusion_texture = texture_index,
            }

            let reference = if let Some(path_buf) = resolved_path.take() {
                let canonical = path_buf.canonicalize().unwrap_or_else(|_| path_buf.clone());
                Some(MaterialTextureReference::new(
                    Some(canonical),
                    slot_data.name.clone(),
                ))
            } else if slot_data.name.is_some() {
                let mut reference = MaterialTextureReference::default();
                reference.set_display_name(slot_data.name.clone());
                Some(reference)
            } else {
                None
            };

            references.push((slot, reference));
        }

        (material, kind, references)
    }

    pub fn apply_metadata_to_asset(&self, asset: &mut MaterialAsset) {
        asset.set_kind(self.kind.to_material_kind());
        for slot in MaterialTextureSlot::all() {
            let slot_data = self.texture_slot(slot);
            if let Some(path) = slot_data.path.as_ref() {
                let resolved = resolve_project_path(path);
                let canonical = resolved.canonicalize().unwrap_or_else(|_| resolved.clone());
                asset.set_texture_reference(
                    slot,
                    MaterialTextureReference::new(Some(canonical), slot_data.name.clone()),
                );
            } else if slot_data.name.is_some() {
                let mut reference = MaterialTextureReference::default();
                reference.set_display_name(slot_data.name.clone());
                asset.set_texture_reference(slot, reference);
            } else {
                asset.clear_texture_reference(slot);
            }
        }
    }
}

impl From<SerializedMaterial> for Material {
    fn from(serialized: SerializedMaterial) -> Self {
        Material {
            base_color: serialized.base_color,
            flags: MaterialFlags::from_bits(serialized.flags),
            base_color_texture: serialized
                .base_color_texture
                .index
                .unwrap_or(DEFAULT_WHITE_TEXTURE_INDEX),
            metallic_roughness_texture: serialized
                .metallic_roughness_texture
                .index
                .unwrap_or(DEFAULT_METALLIC_ROUGHNESS_TEXTURE_INDEX),
            normal_texture: serialized
                .normal_texture
                .index
                .unwrap_or(DEFAULT_NORMAL_TEXTURE_INDEX),
            emissive_texture: serialized
                .emissive_texture
                .index
                .unwrap_or(DEFAULT_WHITE_TEXTURE_INDEX),
            occlusion_texture: serialized
                .occlusion_texture
                .index
                .unwrap_or(DEFAULT_WHITE_TEXTURE_INDEX),
            metallic_factor: serialized.metallic_factor,
            roughness_factor: serialized.roughness_factor,
            emissive_strength: serialized.emissive_strength,
        }
    }
}
