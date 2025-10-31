use std::collections::HashMap;
use std::path::PathBuf;

use super::{textures::ImportedTexture, SceneImportDevice, SceneLoadContext};
use crate::asset::{Handle, MaterialAsset, MaterialTextureReference, MaterialTextureSlot};
use crate::project::resolve_project_path;
use crate::renderer::Material;

pub(super) struct MaterialLoadResult {
    pub material_handles: Vec<Handle<MaterialAsset>>,
    pub default_material: Handle<MaterialAsset>,
}

pub(super) fn load_materials<D: SceneImportDevice>(
    ctx: &mut SceneLoadContext<'_, D>,
    document: &gltf::Document,
    textures: &[ImportedTexture],
) -> Result<MaterialLoadResult, String> {
    log::info!("Loading materials...");

    let mut material_handles = Vec::new();
    let mut material_usage: HashMap<usize, Vec<(usize, usize)>> = HashMap::new();

    for node in document.nodes() {
        if let Some(mesh) = node.mesh() {
            for (primitive_index, primitive) in mesh.primitives().enumerate() {
                if let Some(material) = primitive.material().index() {
                    material_usage
                        .entry(material)
                        .or_default()
                        .push((node.index(), primitive_index));
                }
            }
        }
    }

    for (material_index, gltf_mat) in document.materials().enumerate() {
        let mat_name = gltf_mat.name().unwrap_or("Unnamed");
        let pbr = gltf_mat.pbr_metallic_roughness();

        let base_color = pbr.base_color_factor();
        let base_color_u8 = [
            (base_color[0] * 255.0) as u8,
            (base_color[1] * 255.0) as u8,
            (base_color[2] * 255.0) as u8,
            (base_color[3] * 255.0) as u8,
        ];

        let mut material = Material::new(base_color_u8)
            .with_metallic(pbr.metallic_factor())
            .with_roughness(pbr.roughness_factor());

        let mut base_color_ref: Option<&ImportedTexture> = None;
        let mut metallic_ref: Option<&ImportedTexture> = None;
        let mut normal_ref: Option<&ImportedTexture> = None;
        let mut emissive_ref: Option<&ImportedTexture> = None;
        let mut occlusion_ref: Option<&ImportedTexture> = None;

        if let Some(info) = pbr.base_color_texture() {
            let tex_index = info.texture().index();
            if let Some(texture) = textures.get(tex_index) {
                material = material.with_base_color_texture(texture.index);
                base_color_ref = Some(texture);
            }
        }

        if let Some(info) = pbr.metallic_roughness_texture() {
            let tex_index = info.texture().index();
            if let Some(texture) = textures.get(tex_index) {
                material = material.with_metallic_roughness_texture(texture.index);
                metallic_ref = Some(texture);
            }
        }

        if let Some(normal) = gltf_mat.normal_texture() {
            let tex_index = normal.texture().index();
            if let Some(texture) = textures.get(tex_index) {
                material = material.with_normal_texture(texture.index);
                normal_ref = Some(texture);
            }
        }

        if let Some(emissive) = gltf_mat.emissive_texture() {
            let tex_index = emissive.texture().index();
            if let Some(texture) = textures.get(tex_index) {
                material = material.with_emissive_texture(texture.index);
                emissive_ref = Some(texture);
            }
        }

        let emissive = gltf_mat.emissive_factor();
        let emissive_strength = (emissive[0] + emissive[1] + emissive[2]) / 3.0;
        if emissive_strength > 0.0 {
            material = material.with_emissive(emissive_strength);
        }

        if let Some(occlusion) = gltf_mat.occlusion_texture() {
            let tex_index = occlusion.texture().index();
            if let Some(texture) = textures.get(tex_index) {
                material = material.with_occlusion_texture(texture.index);
                occlusion_ref = Some(texture);
            }
        }

        material = match gltf_mat.alpha_mode() {
            gltf::material::AlphaMode::Opaque => material,
            gltf::material::AlphaMode::Mask | gltf::material::AlphaMode::Blend => {
                material.with_alpha()
            }
        };

        log::debug!(
            "  Material '{}': metallic={:.2}, roughness={:.2}",
            mat_name,
            pbr.metallic_factor(),
            pbr.roughness_factor()
        );

        let resolved_binding = ctx.material_meta.and_then(|meta| {
            if let Some(usages) = material_usage.get(&material_index) {
                for &(node_index, primitive_index) in usages {
                    if let Some(path) = meta.lookup_material_path(
                        Some(node_index),
                        Some(primitive_index),
                        Some(material_index),
                    ) {
                        return Some(path.clone());
                    }

                    if let Some(path) =
                        meta.lookup_material_path(Some(node_index), None, Some(material_index))
                    {
                        return Some(path.clone());
                    }

                    if let Some(path) =
                        meta.lookup_material_path(None, Some(primitive_index), Some(material_index))
                    {
                        return Some(path.clone());
                    }
                }
            }

            meta.lookup_material_path(None, None, Some(material_index))
                .cloned()
        });

        let canonical_path = resolved_binding
            .map(|path| resolve_project_path(&path))
            .unwrap_or_else(|| {
                PathBuf::from(format!(
                    "{}#material{}",
                    ctx.source_path().display(),
                    material_index
                ))
            });

        let mut asset = MaterialAsset::from_material(material, canonical_path);

        let mut apply_reference = |slot: MaterialTextureSlot, texture: Option<&ImportedTexture>| {
            if let Some(texture) = texture {
                if texture.canonical_path.is_some() || texture.display_name.is_some() {
                    asset.set_texture_reference(
                        slot,
                        MaterialTextureReference::new(
                            texture.canonical_path.clone(),
                            texture.display_name.clone(),
                        ),
                    );
                }
            }
        };

        apply_reference(MaterialTextureSlot::BaseColor, base_color_ref);
        apply_reference(MaterialTextureSlot::MetallicRoughness, metallic_ref);
        apply_reference(MaterialTextureSlot::Normal, normal_ref);
        apply_reference(MaterialTextureSlot::Emissive, emissive_ref);
        apply_reference(MaterialTextureSlot::Occlusion, occlusion_ref);

        let handle = {
            let scene = &mut *ctx.scene;
            scene.assets.insert_material_asset(asset)
        };
        material_handles.push(handle);
    }

    let default_path = PathBuf::from(format!("{}#default", ctx.source_path().display()));
    let default_material = {
        let scene = &mut *ctx.scene;
        scene
            .assets
            .insert_material_asset(MaterialAsset::from_material(Material::pbr(), default_path))
    };

    log::info!("Loaded {} materials", material_handles.len());

    Ok(MaterialLoadResult {
        material_handles,
        default_material,
    })
}
