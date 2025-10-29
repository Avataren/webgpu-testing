use std::collections::BTreeSet;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use pollster::block_on;
use tempfile::tempdir;
use wgpu::{
    Backends, DeviceDescriptor, Features, Instance, InstanceDescriptor, Limits, MemoryHints,
    PowerPreference, RequestAdapterOptions, Trace,
};

use wgpu_cube::asset::{MaterialTextureSlot, Mesh};
use wgpu_cube::io::percent_decode_uri;
use wgpu_cube::project::{set_active_project_root, ProjectManifest, ProjectMetadata, CONTENT_DIR};
use wgpu_cube::renderer::Vertex;
use wgpu_cube::scene::{
    components::MeshComponent,
    gltf_package::{PackagedGltfDescriptor, PackagedMesh, PackagedScene, PACKAGED_GLTF_VERSION},
    loader::SceneImportDevice,
    Scene, SceneAssetBundle, SceneLoader,
};

struct HeadlessDevice {
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
}

impl HeadlessDevice {
    fn new(label: &str) -> Option<Self> {
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
                eprintln!("Skipping packaged glTF test: failed to request adapter ({err})");
                return None;
            }
        };

        let (device, queue) = block_on(adapter.request_device(&DeviceDescriptor {
            label: Some(label),
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

fn collect_gltf_dependencies(path: &Path) -> BTreeSet<String> {
    let mut dependencies = BTreeSet::new();
    let document = gltf::Gltf::open(path).expect("glTF should parse");

    for buffer in document.buffers() {
        if let gltf::buffer::Source::Uri(uri) = buffer.source() {
            let trimmed = uri.trim();
            if !trimmed.starts_with("data:") {
                dependencies.insert(trimmed.to_string());
            }
        }
    }

    for image in document.images() {
        match image.source() {
            gltf::image::Source::View { .. } => {}
            gltf::image::Source::Uri { uri, .. } => {
                let trimmed = uri.trim();
                if !trimmed.starts_with("data:") {
                    dependencies.insert(trimmed.to_string());
                }
            }
        }
    }

    dependencies
}

fn safe_join(base: &Path, relative: &Path, original: &str) -> PathBuf {
    let mut result = base.to_path_buf();
    let mut depth = 0usize;
    for component in relative.components() {
        match component {
            Component::Normal(part) => {
                result.push(part);
                depth += 1;
            }
            Component::CurDir => {}
            Component::ParentDir => {
                if depth == 0 {
                    panic!("dependency {original:?} escapes package directory");
                }
                result.pop();
                depth -= 1;
            }
            Component::RootDir | Component::Prefix(_) => {
                panic!("dependency {original:?} attempts to escape package directory");
            }
        }
    }

    result
}

fn package_gltf_into_project(
    project_dir: &Path,
    destination_root: &Path,
    source_path: &Path,
    device: &mut impl SceneImportDevice,
) -> (PathBuf, PathBuf, SceneAssetBundle) {
    assert!(destination_root.starts_with(project_dir.join(CONTENT_DIR)));

    let source_path = source_path.to_path_buf();
    let base_name = source_path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("asset");

    let mut asset_folder = destination_root.join(base_name);
    let mut counter = 1usize;
    while asset_folder.exists() {
        asset_folder = destination_root.join(format!("{base_name}_{counter:02}"));
        counter += 1;
    }

    fs::create_dir_all(&asset_folder).expect("package directory should be created");

    let dependencies = collect_gltf_dependencies(&source_path);
    let source_dir = source_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));

    let mut mapping = Vec::new();

    for uri in dependencies {
        let decoded = percent_decode_uri(&uri).expect("uri should decode");
        if decoded.contains("://") || decoded.starts_with("//") {
            panic!("unsupported external dependency {uri}");
        }

        if decoded.starts_with("data:") {
            continue;
        }

        let relative_path = Path::new(&decoded);
        if relative_path.is_absolute() {
            panic!("absolute dependency {uri} is unsupported");
        }

        let source_dependency = source_dir.join(relative_path);
        let canonical_source = source_dependency
            .canonicalize()
            .unwrap_or(source_dependency.clone());
        let destination_dependency = safe_join(&asset_folder, relative_path, &uri);
        if let Some(parent) = destination_dependency.parent() {
            fs::create_dir_all(parent).expect("dependency directory should exist");
        }
        fs::copy(&source_dependency, &destination_dependency)
            .expect("dependency copy should succeed");

        let project_relative = destination_dependency
            .strip_prefix(project_dir)
            .expect("package path should reside within project")
            .to_path_buf();
        mapping.push((canonical_source, project_relative));
    }

    let file_name = source_path.file_name().expect("gltf should have file name");
    let descriptor_name = format!("{}.import", file_name.to_string_lossy());
    let descriptor_absolute = asset_folder.join(&descriptor_name);
    let project_relative_package = asset_folder
        .strip_prefix(project_dir)
        .expect("package dir should be inside project")
        .to_path_buf();
    let descriptor_relative = descriptor_absolute
        .strip_prefix(project_dir)
        .expect("placeholder should reside within project")
        .to_path_buf();

    let mut bundle = SceneLoader::load_gltf_asset(&source_path, device, 1.0)
        .expect("load glTF asset for packaging");

    let mut lookup = std::collections::HashMap::new();
    for (original, packaged) in &mapping {
        lookup.insert(original.clone(), packaged.clone());
    }

    for entity in &mut bundle.asset.entities {
        if entity.gltf_source.is_some() {
            entity.gltf_source = Some(descriptor_relative.clone());
        }

        if let Some(material) = entity.material_data.as_mut() {
            for slot in MaterialTextureSlot::all() {
                let slot_data = match slot {
                    MaterialTextureSlot::BaseColor => &mut material.base_color_texture,
                    MaterialTextureSlot::MetallicRoughness => {
                        &mut material.metallic_roughness_texture
                    }
                    MaterialTextureSlot::Normal => &mut material.normal_texture,
                    MaterialTextureSlot::Emissive => &mut material.emissive_texture,
                    MaterialTextureSlot::Occlusion => &mut material.occlusion_texture,
                };

                let Some(existing_path) = slot_data.path.as_ref() else {
                    continue;
                };

                let resolved = if existing_path.is_absolute() {
                    existing_path.clone()
                } else {
                    source_path
                        .parent()
                        .map(|parent| parent.join(existing_path))
                        .unwrap_or_else(|| existing_path.clone())
                };
                let canonical = resolved.canonicalize().unwrap_or(resolved);

                if let Some(new_rel) = lookup.get(&canonical) {
                    slot_data.path = Some(new_rel.clone());
                }
            }
        }
    }

    let mut stored_asset = bundle.asset.clone();
    stored_asset.mesh_data.clear();

    let scene_rel = PathBuf::from("scene.asset.json");
    let scene_abs = asset_folder.join(&scene_rel);
    let scene_json = stored_asset.to_json().expect("serialize scene asset");
    fs::write(&scene_abs, scene_json).expect("write packaged scene asset");

    let mut meshes = Vec::new();
    if !bundle.asset.mesh_data.is_empty() {
        let mesh_dir_rel = PathBuf::from("meshes");
        let mesh_dir_abs = asset_folder.join(&mesh_dir_rel);
        fs::create_dir_all(&mesh_dir_abs).expect("mesh dir exists");

        for (index, data) in bundle.asset.mesh_data.iter().enumerate() {
            let mesh_rel = mesh_dir_rel.join(format!("mesh_{index:04}.json"));
            let mesh_abs = asset_folder.join(&mesh_rel);
            let mesh_json = serde_json::to_string_pretty(data).expect("serialize mesh");
            fs::write(&mesh_abs, mesh_json).expect("write mesh payload");
            meshes.push(PackagedMesh {
                index,
                path: mesh_rel,
            });
        }
    }

    let descriptor = PackagedGltfDescriptor {
        version: PACKAGED_GLTF_VERSION,
        source: None,
        scene: Some(PackagedScene {
            json: scene_rel,
            meshes,
        }),
    };

    let descriptor_json = serde_json::to_string_pretty(&descriptor).expect("serialize descriptor");
    fs::write(&descriptor_absolute, descriptor_json).expect("write descriptor");

    (project_relative_package, descriptor_relative, bundle)
}

#[test]
fn packaged_gltf_roundtrip_without_source() {
    let Some(mut headless) = HeadlessDevice::new("headless-packaged-gltf") else {
        eprintln!("Skipping packaged glTF regression test: headless device unavailable");
        return;
    };

    let project_dir = tempdir().expect("temp project");
    let project_root = project_dir.path();
    let content_root = project_root.join(CONTENT_DIR);
    fs::create_dir_all(&content_root).expect("content directory should exist");

    let source_dir = tempdir().expect("temp gltf source");
    let source_root = source_dir.path();
    let gltf_source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("web/assets/animated/AnimatedColorsCube.gltf");
    let bin_source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("web/assets/animated/AnimatedColorsCube.bin");
    let texture_source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("web/assets/animated/AnimatedCube_BaseColor.png");

    fs::copy(&gltf_source, source_root.join("AnimatedColorsCube.gltf")).expect("copy gltf");
    fs::copy(&bin_source, source_root.join("AnimatedColorsCube.bin")).expect("copy bin");
    fs::copy(
        &texture_source,
        source_root.join("AnimatedCube_BaseColor.png"),
    )
    .expect("copy texture");

    set_active_project_root(Some(project_root.to_path_buf()));

    let gltf_path = source_root.join("AnimatedColorsCube.gltf");
    let (package_rel, descriptor_rel, mut bundle) =
        package_gltf_into_project(project_root, &content_root, &gltf_path, &mut headless);

    let package_dir = project_root.join(&package_rel);

    let mut scene = Scene::new();
    let registration = bundle.register_resources(&mut headless, &mut scene.assets);
    if registration.textures_changed() {
        headless
            .queue()
            .submit(std::iter::empty::<wgpu::CommandBuffer>());
    }
    let node = scene.instantiate_asset_with_renderer(&bundle.asset, None, &mut headless);
    scene.set_main_scene(node);

    let manifest =
        ProjectManifest::capture(&scene, ProjectMetadata::default()).expect("manifest capture");
    manifest.save_to_dir(project_root).expect("save manifest");

    drop(scene);
    drop(bundle);

    fs::remove_dir_all(source_root).expect("remove source directory");

    let descriptor_path = project_root.join(&descriptor_rel);
    let mut packaged_bundle = SceneLoader::load_gltf_asset(&descriptor_path, &mut headless, 1.0)
        .expect("load packaged descriptor");
    let mut packaged_scene = Scene::new();
    let packaged_registration =
        packaged_bundle.register_resources(&mut headless, &mut packaged_scene.assets);
    if packaged_registration.textures_changed() {
        headless
            .queue()
            .submit(std::iter::empty::<wgpu::CommandBuffer>());
    }
    let packaged_node =
        packaged_scene.instantiate_asset_with_renderer(&packaged_bundle.asset, None, &mut headless);
    packaged_scene.set_main_scene(packaged_node);

    let packaged_mesh_count = packaged_scene
        .main_world()
        .query::<&MeshComponent>()
        .iter()
        .count();
    assert!(
        packaged_mesh_count > 0,
        "packaged descriptor should produce meshes"
    );
    assert!(
        !packaged_scene.animations().is_empty(),
        "packaged descriptor should include animation clips"
    );

    drop(packaged_scene);
    drop(packaged_bundle);

    let loaded = ProjectManifest::load_from_dir(project_root).expect("load manifest");
    let mut restored_scene = Scene::new();
    let textures_changed = loaded
        .instantiate_into(&mut restored_scene, &mut headless, project_root)
        .expect("instantiate project");
    if textures_changed {
        headless
            .queue()
            .submit(std::iter::empty::<wgpu::CommandBuffer>());
    }

    let mesh_count = restored_scene
        .main_world()
        .query::<&MeshComponent>()
        .iter()
        .count();
    assert!(mesh_count > 0, "restored scene should have meshes");
    assert!(
        !restored_scene.animations().is_empty(),
        "restored scene should include animation clips"
    );
    assert!(
        !restored_scene.animation_states().is_empty(),
        "restored scene should include animation states"
    );

    assert!(package_dir.exists(), "packaged directory should remain");
}
