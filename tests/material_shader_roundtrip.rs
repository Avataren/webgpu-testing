use std::path::PathBuf;

use wgpu_cube::asset::{Assets, MaterialAsset, MaterialKind};
use wgpu_cube::renderer::material::Material;
use wgpu_cube::scene::{SerializedMaterial, SerializedMaterialKind};

#[test]
fn serialized_shader_material_roundtrips_metadata() {
    let mut shader_asset =
        MaterialAsset::shader(Material::pbr(), PathBuf::from("shader_roundtrip.mat.json"));

    let custom_wgsl = "@fragment fn fragment_main() -> @location(0) vec4<f32> {\n    return vec4<f32>(0.2, 0.4, 0.8, 1.0);\n}".to_string();

    {
        let metadata = shader_asset
            .shader_metadata_mut()
            .expect("shader asset should provide metadata");
        metadata.set_wgsl_source(custom_wgsl.clone());
        metadata.set_needs_lighting_include(true);
    }

    let serialized = SerializedMaterial::from_material_asset(&shader_asset);

    match &serialized.kind {
        SerializedMaterialKind::Shader {
            wgsl_source,
            needs_lighting_include,
        } => {
            assert_eq!(wgsl_source, &custom_wgsl, "WGSL source should persist");
            assert!(
                *needs_lighting_include,
                "lighting include flag should persist"
            );
        }
        other => panic!("expected shader kind, got {other:?}"),
    }

    let mut assets = Assets::default();
    let (material, kind, _references) = serialized.resolve_material(&mut assets, None);

    match kind {
        MaterialKind::Shader(metadata) => {
            assert_eq!(
                metadata.wgsl_source(),
                custom_wgsl,
                "resolved metadata should contain WGSL source"
            );
            assert!(
                metadata.needs_lighting_include(),
                "resolved metadata should retain lighting include flag"
            );
        }
        MaterialKind::Pbr => panic!("expected shader metadata to round-trip"),
    }

    let mut restored =
        MaterialAsset::from_material(material, PathBuf::from("restored_shader.mat.json"));
    serialized.apply_metadata_to_asset(&mut restored);

    match restored.kind() {
        MaterialKind::Shader(metadata) => {
            assert_eq!(
                metadata.wgsl_source(),
                custom_wgsl,
                "applied metadata should update WGSL source"
            );
            assert!(
                metadata.needs_lighting_include(),
                "applied metadata should set lighting include flag"
            );
        }
        MaterialKind::Pbr => panic!("expected restored material asset to remain a shader"),
    }
}
