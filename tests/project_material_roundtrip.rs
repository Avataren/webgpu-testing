use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use pollster::block_on;
use tempfile::tempdir;
use wgpu::{
    Backends, DeviceDescriptor, Features, Instance, InstanceDescriptor, Limits, MemoryHints,
    PowerPreference, RequestAdapterOptions, Trace,
};

use wgpu_cube::asset::{MaterialTextureSlot, Mesh};
use wgpu_cube::project::{set_active_project_root, ProjectManifest, ProjectMetadata};
use wgpu_cube::renderer::Vertex;
use wgpu_cube::scene::{
    MaterialComponent, Scene, SceneImportDevice, SceneLoader, SerializedMaterial,
};

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
fn project_manifest_preserves_custom_material_edits() {
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

    let mut scene = Scene::new();
    bundle.register_resources(&mut headless, &mut scene.assets);
    let node = scene.instantiate_asset_with_renderer(&bundle.asset, None, &mut headless);
    scene.set_main_scene(node);

    let manifest = ProjectManifest::capture(&scene, ProjectMetadata::default())
        .expect("capturing manifest should succeed");
    manifest
        .save_to_dir(project_root)
        .expect("manifest should save to disk");

    let materials_dir = project_root.join("content/materials");
    let mat_path = fs::read_dir(&materials_dir)
        .expect("material directory should exist")
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .find(|path| {
            path.extension().and_then(|ext| ext.to_str()) == Some("json")
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .map(|name| name.ends_with(".mat.json"))
                    .unwrap_or(false)
        })
        .expect("expected at least one material asset to be generated");

    let original = fs::read_to_string(&mat_path).expect("material asset should be readable");
    let mut serialized: SerializedMaterial =
        serde_json::from_str(&original).expect("material asset should deserialize");

    let expected_color = [17, 34, 51, 255];
    serialized.base_color = expected_color;
    serialized.metallic_factor = 200;
    serialized.roughness_factor = 25;
    serialized.emissive_strength = 15;
    let texture_name = "Custom Painted".to_string();
    serialized.base_color_texture.name = Some(texture_name.clone());

    let modified = serde_json::to_string_pretty(&serialized)
        .expect("material asset should serialize after edits");
    fs::write(&mat_path, &modified).expect("material asset should be rewritten");

    let loaded = ProjectManifest::load_from_dir(project_root)
        .expect("reloading manifest from disk should succeed");

    let mut restored_scene = Scene::new();
    loaded
        .instantiate_into(&mut restored_scene, &mut headless, project_root)
        .expect("project should instantiate successfully");

    let final_json = fs::read_to_string(&mat_path).expect("material asset should remain readable");
    assert_eq!(
        final_json, modified,
        "instantiation should not overwrite customized material asset"
    );

    let target_file_name = mat_path
        .file_name()
        .expect("material asset should have a file name")
        .to_owned();

    let mut found_custom_material = false;
    for (_, (material_component,)) in restored_scene
        .world()
        .query::<(&MaterialComponent,)>()
        .iter()
    {
        if let Some(asset) = restored_scene.assets.material(material_component.0) {
            if asset
                .canonical_path()
                .file_name()
                .map(|name| name == target_file_name)
                .unwrap_or(false)
            {
                assert_eq!(
                    asset.material().base_color,
                    expected_color,
                    "base color override should persist"
                );
                assert_eq!(
                    asset.material().metallic_factor,
                    serialized.metallic_factor,
                    "metallic factor override should persist"
                );
                let reference = asset
                    .texture_reference(MaterialTextureSlot::BaseColor)
                    .expect("base color texture reference should exist");
                assert_eq!(
                    reference.display_name(),
                    Some(texture_name.as_str()),
                    "texture display name override should persist"
                );
                found_custom_material = true;
                break;
            }
        }
    }

    assert!(
        found_custom_material,
        "instantiated scene should include the customized material"
    );

    set_active_project_root(None);
}
