use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use base64::encode;
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
    gltf_package::{
        PackagedGltfDescriptor, PackagedMesh, PackagedScene, PackagedTexture, PACKAGED_GLTF_VERSION,
    },
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

    let mut lookup = HashMap::new();
    for (original, packaged) in &mapping {
        lookup.insert(original.clone(), packaged.clone());
    }

    let (document, buffers, _) =
        SceneLoader::import_gltf_native(&source_path).expect("import glTF for packaging");

    let mut image_package_paths: HashMap<usize, PathBuf> = HashMap::new();
    let mut image_payloads: Vec<(PathBuf, Vec<u8>)> = Vec::new();
    let mut used_texture_names: HashSet<String> = HashSet::new();
    let texture_dir_rel = PathBuf::from("textures");

    for image in document.images() {
        match image.source() {
            gltf::image::Source::View { view, mime_type } => {
                let buffer = &buffers[view.buffer().index()].0;
                let start = view.offset();
                let end = start + view.length();
                if end > buffer.len() {
                    panic!("embedded texture view {} exceeds buffer", image.index());
                }

                let payload = buffer[start..end].to_vec();
                let extension = extension_for_mime_type(Some(mime_type));
                let file_name = unique_texture_file_name(
                    &mut used_texture_names,
                    image.name(),
                    image.index(),
                    &extension,
                );
                let package_relative = texture_dir_rel.join(&file_name);
                image_package_paths.insert(image.index(), package_relative.clone());
                image_payloads.push((package_relative, payload));
            }
            gltf::image::Source::Uri { uri, mime_type } => {
                let trimmed = uri.trim();
                if !trimmed.starts_with("data:") {
                    continue;
                }

                let (inferred_mime, payload) = decode_embedded_data_uri(trimmed);
                let mime = mime_type
                    .map(|value| value.to_string())
                    .or(inferred_mime)
                    .unwrap_or_else(|| "application/octet-stream".to_string());
                let extension = extension_for_mime_type(Some(&mime));
                let file_name = unique_texture_file_name(
                    &mut used_texture_names,
                    image.name(),
                    image.index(),
                    &extension,
                );
                let package_relative = texture_dir_rel.join(&file_name);
                image_package_paths.insert(image.index(), package_relative.clone());
                image_payloads.push((package_relative, payload));
            }
        }
    }

    let mut texture_package_paths: HashMap<usize, PathBuf> = HashMap::new();
    let mut texture_project_paths: HashMap<usize, PathBuf> = HashMap::new();

    for texture in document.textures() {
        let image_index = texture.source().index();
        if let Some(package_relative) = image_package_paths.get(&image_index) {
            let absolute = asset_folder.join(package_relative);
            let project_relative = absolute
                .strip_prefix(project_dir)
                .expect("packaged texture should remain inside project")
                .to_path_buf();
            texture_package_paths.insert(texture.index(), package_relative.clone());
            texture_project_paths.insert(texture.index(), project_relative);
        }
    }

    let materials: Vec<_> = document.materials().collect();
    let mut packaged_local_textures: BTreeMap<u32, PathBuf> = BTreeMap::new();

    for entity in &mut bundle.asset.entities {
        if entity.gltf_source.is_some() {
            entity.gltf_source = Some(descriptor_relative.clone());
        }

        if let Some(material) = entity.material_data.as_mut() {
            let gltf_material = entity.gltf_material.and_then(|index| materials.get(index));
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

                if slot_data.path.is_none() {
                    if let Some(gltf_material) = gltf_material {
                        if let Some(texture_index) = texture_index_for_slot(gltf_material, slot) {
                            if let Some(project_relative) =
                                texture_project_paths.get(&texture_index)
                            {
                                slot_data.path = Some(project_relative.clone());
                                if let Some(local_index) = slot_data.index {
                                    if let Some(package_relative) =
                                        texture_package_paths.get(&texture_index)
                                    {
                                        packaged_local_textures
                                            .entry(local_index)
                                            .or_insert_with(|| package_relative.clone());
                                    }
                                }
                                continue;
                            }
                        }
                    }
                }

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

    for (relative, payload) in image_payloads {
        let absolute = asset_folder.join(&relative);
        if let Some(parent) = absolute.parent() {
            fs::create_dir_all(parent).expect("texture dir exists");
        }
        fs::write(&absolute, &payload).expect("write packaged texture");
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

    let textures: Vec<PackagedTexture> = packaged_local_textures
        .into_iter()
        .map(|(index, path)| PackagedTexture { index, path })
        .collect();

    let descriptor = PackagedGltfDescriptor {
        version: PACKAGED_GLTF_VERSION,
        source: None,
        scene: Some(PackagedScene {
            json: scene_rel,
            meshes,
            textures,
        }),
    };

    let descriptor_json = serde_json::to_string_pretty(&descriptor).expect("serialize descriptor");
    fs::write(&descriptor_absolute, descriptor_json).expect("write descriptor");

    (project_relative_package, descriptor_relative, bundle)
}

fn texture_index_for_slot(material: &gltf::Material, slot: MaterialTextureSlot) -> Option<usize> {
    match slot {
        MaterialTextureSlot::BaseColor => material
            .pbr_metallic_roughness()
            .base_color_texture()
            .map(|info| info.texture().index()),
        MaterialTextureSlot::MetallicRoughness => material
            .pbr_metallic_roughness()
            .metallic_roughness_texture()
            .map(|info| info.texture().index()),
        MaterialTextureSlot::Normal => material.normal_texture().map(|info| info.texture().index()),
        MaterialTextureSlot::Emissive => material
            .emissive_texture()
            .map(|info| info.texture().index()),
        MaterialTextureSlot::Occlusion => material
            .occlusion_texture()
            .map(|info| info.texture().index()),
    }
}

fn sanitize_texture_stem(name: &str) -> String {
    let mut stem = String::new();
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' {
            stem.push(ch);
        } else if !stem.ends_with('_') {
            stem.push('_');
        }
    }

    let trimmed = stem.trim_matches('_');
    if trimmed.is_empty() {
        String::new()
    } else {
        trimmed.to_string()
    }
}

fn unique_texture_file_name(
    used: &mut HashSet<String>,
    original_name: Option<&str>,
    index: usize,
    extension: &str,
) -> String {
    let base = original_name
        .map(sanitize_texture_stem)
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| format!("embedded_{index:04}"));

    let mut candidate = format!("{base}.{extension}");
    let mut suffix = 1usize;
    while !used.insert(candidate.clone()) {
        candidate = format!("{base}_{suffix:02}.{}", extension);
        suffix += 1;
    }

    candidate
}

fn extension_for_mime_type(mime: Option<&str>) -> String {
    let mime = mime
        .map(|value| value.to_ascii_lowercase())
        .unwrap_or_else(|| "application/octet-stream".to_string());

    let ext = match mime.as_str() {
        "image/png" => "png",
        "image/jpeg" | "image/jpg" => "jpg",
        "image/webp" => "webp",
        "image/bmp" => "bmp",
        "image/gif" => "gif",
        "image/tga" => "tga",
        "image/vnd.microsoft.icon" | "image/x-icon" => "ico",
        "image/ktx" => "ktx",
        "image/ktx2" => "ktx2",
        "image/vnd.ms-dds" | "image/vnd-ms.dds" | "image/x-dds" | "application/vnd.ms-dds" => "dds",
        _ => "bin",
    };

    ext.to_string()
}

fn decode_embedded_data_uri(uri: &str) -> (Option<String>, Vec<u8>) {
    let rest = uri
        .strip_prefix("data:")
        .unwrap_or_else(|| panic!("{uri:?} is not a data URI"));
    let mut parts = rest.splitn(2, ',');
    let meta = parts.next().unwrap_or("");
    let data = parts
        .next()
        .unwrap_or_else(|| panic!("data URI {uri:?} missing payload"));

    let mut mime: Option<String> = None;
    let mut is_base64 = false;

    if !meta.is_empty() {
        for token in meta.split(';') {
            if token.eq_ignore_ascii_case("base64") {
                is_base64 = true;
            } else if mime.is_none() && !token.is_empty() {
                mime = Some(token.to_string());
            }
        }
    }

    if !is_base64 {
        panic!("data URI {uri:?} is not base64 encoded");
    }

    let decoded = base64::decode(data).unwrap_or_else(|error| {
        panic!("failed to decode data URI {uri:?}: {error}");
    });

    (mime, decoded)
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

    let gltf_copy_path = source_root.join("AnimatedColorsCube.gltf");
    let texture_copy_path = source_root.join("AnimatedCube_BaseColor.png");
    let texture_bytes = fs::read(&texture_copy_path).expect("read embedded texture source");
    let data_uri = format!("data:image/png;base64,{}", encode(&texture_bytes));
    let mut gltf_json = fs::read_to_string(&gltf_copy_path).expect("read glTF source");
    gltf_json = gltf_json.replace("AnimatedCube_BaseColor.png", &data_uri);
    fs::write(&gltf_copy_path, gltf_json).expect("embed texture in glTF");
    fs::remove_file(&texture_copy_path).expect("remove texture source");

    set_active_project_root(Some(project_root.to_path_buf()));

    let gltf_path = source_root.join("AnimatedColorsCube.gltf");
    let (package_rel, descriptor_rel, mut bundle) =
        package_gltf_into_project(project_root, &content_root, &gltf_path, &mut headless);

    let package_dir = project_root.join(&package_rel);
    let descriptor_path = project_root.join(&descriptor_rel);

    let descriptor_text = fs::read_to_string(&descriptor_path).expect("read packaged descriptor");
    let descriptor: PackagedGltfDescriptor =
        serde_json::from_str(&descriptor_text).expect("parse packaged descriptor");
    let packaged_scene = descriptor
        .scene
        .expect("descriptor should contain scene payload");
    assert!(
        !packaged_scene.textures.is_empty(),
        "packaged descriptor should record embedded textures"
    );
    for texture in &packaged_scene.textures {
        let texture_path = package_dir.join(&texture.path);
        assert!(
            texture_path.exists(),
            "packaged texture {:?} should exist on disk",
            texture_path
        );
    }

    let mut packaged_texture_paths = Vec::new();
    for entity in &bundle.asset.entities {
        if let Some(material) = &entity.material_data {
            for slot in MaterialTextureSlot::all() {
                let slot_data = match slot {
                    MaterialTextureSlot::BaseColor => &material.base_color_texture,
                    MaterialTextureSlot::MetallicRoughness => &material.metallic_roughness_texture,
                    MaterialTextureSlot::Normal => &material.normal_texture,
                    MaterialTextureSlot::Emissive => &material.emissive_texture,
                    MaterialTextureSlot::Occlusion => &material.occlusion_texture,
                };

                if let Some(path) = slot_data.path.as_ref() {
                    packaged_texture_paths.push(path.clone());
                }
            }
        }
    }

    assert!(
        packaged_texture_paths
            .iter()
            .any(|path| path.to_string_lossy().contains("textures")),
        "serialized materials should reference packaged texture paths"
    );
    for relative in packaged_texture_paths
        .iter()
        .filter(|path| path.to_string_lossy().contains("textures"))
    {
        let absolute = project_root.join(relative);
        assert!(
            absolute.exists(),
            "packaged material texture {:?} should exist",
            absolute
        );
    }

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

    let mut packaged_bundle = SceneLoader::load_gltf_asset(&descriptor_path, &mut headless, 1.0)
        .expect("load packaged descriptor");
    let mut packaged_scene = Scene::new();
    let packaged_registration =
        packaged_bundle.register_resources(&mut headless, &mut packaged_scene.assets);
    assert!(
        packaged_registration.textures_changed(),
        "packaged resources should register embedded textures"
    );
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
