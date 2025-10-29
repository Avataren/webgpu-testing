use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use pollster::block_on;
use tempfile::tempdir;
use wgpu::{
    Backends, DeviceDescriptor, Features, Instance, InstanceDescriptor, Limits, MemoryHints,
    PowerPreference, RequestAdapterOptions, Trace,
};

use serde_json::to_value;
use wgpu_cube::asset::Mesh;
use wgpu_cube::project::{
    resolve_project_path, set_active_project_root, ProjectManifest, ProjectMetadata,
};
use wgpu_cube::renderer::Vertex;
use wgpu_cube::scene::{Scene, SceneAsset, SceneImportDevice, SceneLoader, SerializedMaterial};

struct HeadlessDevice {
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
}

impl HeadlessDevice {
    fn new() -> Option<Self> {
        let instance = Instance::new(&InstanceDescriptor {
            backends: Backends::all(),
            ..Default::default()
        });
        let adapter = match block_on(instance.request_adapter(&RequestAdapterOptions {
            power_preference: PowerPreference::LowPower,
            compatible_surface: None,
            force_fallback_adapter: false,
        })) {
            Ok(adapter) => adapter,
            Err(err) => {
                eprintln!("Skipping material roundtrip test: failed to request adapter ({err})");
                return None;
            }
        };

        let (device, queue) = block_on(adapter.request_device(&DeviceDescriptor {
            label: Some("headless-material-roundtrip"),
            required_features: Features::empty(),
            required_limits: Limits::default(),
            memory_hints: MemoryHints::Performance,
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
            trace: Trace::Off,
        }))
        .ok()?;

        Some(Self {
            device: Arc::new(device),
            queue: Arc::new(queue),
        })
    }
}

impl SceneImportDevice for HeadlessDevice {
    fn device(&self) -> &wgpu::Device {
        &self.device
    }

    fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }

    fn create_mesh(&mut self, vertices: &[Vertex], indices: &[u32]) -> Mesh {
        Mesh::from_vertices(&self.device, vertices, indices)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct GltfMaterialKey {
    source: PathBuf,
    node: Option<usize>,
    primitive: Option<usize>,
    material_index: Option<usize>,
}

fn normalize_gltf_source(source: &Path) -> PathBuf {
    let resolved = resolve_project_path(source);
    if resolved.is_absolute() {
        resolved.canonicalize().unwrap_or_else(|_| resolved.clone())
    } else {
        let absolute = resolve_project_path(resolved);
        absolute.canonicalize().unwrap_or_else(|_| absolute.clone())
    }
}

fn collect_material_map(asset: &SceneAsset) -> HashMap<GltfMaterialKey, SerializedMaterial> {
    let mut map = HashMap::new();

    for entity in &asset.entities {
        let Some(source) = entity.gltf_source.as_ref() else {
            continue;
        };
        let Some(material) = entity.material_data.as_ref() else {
            continue;
        };

        let normalized_source = normalize_gltf_source(source);
        let material = material.clone();

        if let (Some(node), Some(primitive)) = (entity.gltf_node, entity.gltf_primitive) {
            map.insert(
                GltfMaterialKey {
                    source: normalized_source.clone(),
                    node: Some(node),
                    primitive: Some(primitive),
                    material_index: None,
                },
                material.clone(),
            );

            if let Some(material_index) = entity.gltf_material {
                map.insert(
                    GltfMaterialKey {
                        source: normalized_source.clone(),
                        node: Some(node),
                        primitive: Some(primitive),
                        material_index: Some(material_index),
                    },
                    material.clone(),
                );
            }
        }

        if let Some(material_index) = entity.gltf_material {
            map.insert(
                GltfMaterialKey {
                    source: normalized_source.clone(),
                    node: None,
                    primitive: None,
                    material_index: Some(material_index),
                },
                material.clone(),
            );
        }
    }

    map
}

fn copy_directory(src: &Path, dst: &Path) {
    fs::create_dir_all(dst).expect("failed to create destination directory");

    for entry in fs::read_dir(src).expect("failed to list source directory") {
        let entry = entry.expect("failed to access entry");
        let path = entry.path();
        let target = dst.join(entry.file_name());
        let file_type = entry.file_type().expect("failed to read entry type");

        if file_type.is_dir() {
            copy_directory(&path, &target);
        } else if file_type.is_file() {
            fs::copy(&path, &target).expect("failed to copy file");
        }
    }
}

#[test]
fn project_material_roundtrip_preserves_registry() {
    let Some(mut headless) = HeadlessDevice::new() else {
        eprintln!("Skipping material roundtrip test: no suitable headless adapter available");
        return;
    };

    let temp_dir = tempdir().expect("temporary directory should be created");
    let project_root = temp_dir.path();
    let asset_source = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("web/assets/avocado");
    let asset_target = project_root.join("assets/avocado");
    copy_directory(&asset_source, &asset_target);

    set_active_project_root(Some(project_root.to_path_buf()));

    let gltf_path = PathBuf::from("assets/avocado/Avocado.gltf");
    let mut bundle = SceneLoader::load_gltf_asset(&gltf_path, &mut headless, 1.0)
        .expect("failed to import test glTF");

    let mutated_color = [12, 34, 220, 255];
    let mutated_metallic = 37;
    let mutated_roughness = 83;
    let mutated_emissive = 21;
    let mutated_texture_name = "Roundtrip Painted".to_string();

    let mut mutated = false;
    for entity in &mut bundle.asset.entities {
        if let Some(material) = entity.material_data.as_mut() {
            material.base_color = mutated_color;
            material.metallic_factor = mutated_metallic;
            material.roughness_factor = mutated_roughness;
            material.emissive_strength = mutated_emissive;
            material.base_color_texture.name = Some(mutated_texture_name.clone());
            mutated = true;
            break;
        }
    }

    assert!(mutated, "expected at least one material to mutate");

    let mut scene = Scene::new();
    bundle.register_resources(&mut headless, &mut scene.assets);
    let node = scene.instantiate_asset_with_renderer(&bundle.asset, None, &mut headless);
    scene.set_main_scene(node);

    let manifest = ProjectManifest::capture(&scene, ProjectMetadata::default())
        .expect("capturing manifest should succeed");
    let before_map = collect_material_map(&manifest.scene);
    assert!(
        !before_map.is_empty(),
        "expected material registry to contain entries"
    );

    manifest
        .save_to_dir(project_root)
        .expect("manifest should save to disk");

    let loaded = ProjectManifest::load_from_dir(project_root)
        .expect("reloading manifest from disk should succeed");

    let mut restored_scene = Scene::new();
    loaded
        .instantiate_into(&mut restored_scene, &mut headless, project_root)
        .expect("project should instantiate successfully");

    let recaptured = ProjectManifest::capture(&restored_scene, loaded.metadata.clone())
        .expect("recapturing manifest should succeed");
    let after_map = collect_material_map(&recaptured.scene);

    for (key, before_material) in &before_map {
        let after_material = after_map
            .get(key)
            .unwrap_or_else(|| panic!("missing material for key {:?}", key));

        assert_eq!(
            after_material.base_color, before_material.base_color,
            "base color should remain consistent for {:?}",
            key
        );
        assert_eq!(
            after_material.base_color_texture.path, before_material.base_color_texture.path,
            "base color texture path should remain consistent for {:?}",
            key
        );
    }

    let mutated_keys: Vec<_> = before_map
        .iter()
        .filter_map(|(key, material)| (material.base_color == mutated_color).then(|| key.clone()))
        .collect();

    assert!(
        !mutated_keys.is_empty(),
        "expected to locate keys for the mutated material"
    );

    for key in mutated_keys {
        let before_material = before_map.get(&key).expect("mutated material present");
        let after_material = after_map.get(&key).expect("mutated material persisted");

        assert_eq!(
            after_material.metallic_factor, mutated_metallic,
            "metallic factor override should persist for {:?}",
            key
        );
        assert_eq!(
            after_material.roughness_factor, mutated_roughness,
            "roughness override should persist for {:?}",
            key
        );
        assert_eq!(
            after_material.emissive_strength, mutated_emissive,
            "emissive strength override should persist for {:?}",
            key
        );
        assert_eq!(
            after_material.base_color_texture.name.as_deref(),
            Some(mutated_texture_name.as_str()),
            "base color texture name should persist for {:?}",
            key
        );
        assert_eq!(
            after_material.base_color_texture.path, before_material.base_color_texture.path,
            "mutated material should retain its texture path for {:?}",
            key
        );
    }

    let serialized_scene = recaptured
        .scene
        .to_json()
        .expect("serializing captured scene should succeed");
    let mut deserialized_scene = SceneAsset::from_json(&serialized_scene)
        .expect("deserializing captured scene should succeed");

    for entity in &mut deserialized_scene.entities {
        entity.material_data = None;
    }

    deserialized_scene
        .persist_material_assets(project_root)
        .expect("persisting materials should succeed");

    let recovered_from_disk = collect_material_map(&deserialized_scene);

    for (key, expected_material) in &after_map {
        let recovered = recovered_from_disk
            .get(key)
            .unwrap_or_else(|| panic!("missing recovered material for key {:?}", key));

        let recovered_json = to_value(recovered).expect("material should serialize");
        let expected_json = to_value(expected_material).expect("material should serialize");

        assert_eq!(
            recovered_json, expected_json,
            "recovered material overrides should match for {:?}",
            key
        );
    }

    set_active_project_root(None);
}
