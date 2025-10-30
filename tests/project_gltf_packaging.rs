use std::collections::{BTreeSet, HashMap};
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
    // Use a tolerant JSON parse to extract buffer/image URIs. Some glTF files
    // may contain non-standard extensions or variants that `gltf::Gltf::open`
    // strictly rejects; for dependency discovery we only need the URI strings.
    let mut dependencies = BTreeSet::new();
    let text = std::fs::read_to_string(path).expect("glTF should be readable");
    let json: serde_json::Value = serde_json::from_str(&text).expect("glTF should be valid JSON");

    if let Some(buffers) = json.get("buffers").and_then(|v| v.as_array()) {
        for buffer in buffers.iter() {
            if let Some(uri) = buffer.get("uri").and_then(|u| u.as_str()) {
                let trimmed = uri.trim();
                // Skip embedded data URIs, base64-like payloads, comma-containing
                // fragments, or absurdly long URIs which are not filesystem
                // dependencies. This is defensive: some glTF producers percent-
                // encode data: payloads or include long inline blobs that should
                // not be treated as paths.
                if trimmed.starts_with("data:")
                    || trimmed.contains("data:")
                    || trimmed.contains("base64")
                    || trimmed.contains(',')
                    || trimmed.len() > 1024
                {
                    continue;
                }
                dependencies.insert(trimmed.to_string());
            }
        }
    }

    if let Some(images) = json.get("images").and_then(|v| v.as_array()) {
        for image in images.iter() {
            if let Some(uri) = image.get("uri").and_then(|u| u.as_str()) {
                let trimmed = uri.trim();
                // Same defensive filtering as above for image URIs.
                if trimmed.starts_with("data:")
                    || trimmed.contains("data:")
                    || trimmed.contains("base64")
                    || trimmed.contains(',')
                    || trimmed.len() > 1024
                {
                    continue;
                }
                dependencies.insert(trimmed.to_string());
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

fn decode_embedded_data_uri(uri: &str) -> (Option<String>, Vec<u8>) {
    let rest = uri
        .strip_prefix("data:")
        .unwrap_or_else(|| panic!("unsupported data URI {uri}"));

    let mut parts = rest.splitn(2, ',');
    let meta = parts.next().unwrap_or("");
    let data = parts
        .next()
        .unwrap_or_else(|| panic!("data URI {uri} is missing payload"));

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

    assert!(is_base64, "unsupported non-base64 data URI {uri}");

    let decoded = base64::decode(data).expect("base64 data URI should decode");
    (mime, decoded)
}

fn extension_for_mime_type(mime: Option<&str>) -> String {
    let mime = mime
        .map(|value| value.to_ascii_lowercase())
        .unwrap_or_else(|| "application/octet-stream".to_string());

    match mime.as_str() {
        "image/png" => "png".to_string(),
        "image/jpeg" | "image/jpg" => "jpg".to_string(),
        "image/webp" => "webp".to_string(),
        "image/bmp" => "bmp".to_string(),
        "image/gif" => "gif".to_string(),
        "image/tga" => "tga".to_string(),
        "image/vnd.microsoft.icon" | "image/x-icon" => "ico".to_string(),
        "image/ktx" => "ktx".to_string(),
        "image/ktx2" => "ktx2".to_string(),
        "image/vnd.ms-dds" | "image/vnd-ms.dds" | "image/x-dds" | "application/vnd.ms-dds" => {
            "dds".to_string()
        }
        _ => "bin".to_string(),
    }
}

fn texture_file_name(index: usize, extension: &str) -> String {
    format!("embedded_{index:04}.{extension}")
}

// Heuristic: determine whether a URI (possibly percent-encoded) looks like
// an embedded/data payload or an inline base64 fragment. This checks both
// raw and percent-encoded markers (e.g. `data%3A`, `%2C`) to avoid
// decoding large embedded payloads accidentally.
fn looks_like_embedded_payload(uri: &str) -> bool {
    let lower = uri.to_ascii_lowercase();
    if lower.contains("data:") || lower.contains("data%3a") {
        return true;
    }
    if lower.contains("base64") || lower.contains("base64%") {
        return true;
    }
    if lower.contains(',') || lower.contains("%2c") {
        return true;
    }
    false
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
        // Print only a short sample of the URI and its length to avoid
        // accidentally logging very large embedded payloads.
        let sample = &uri[..std::cmp::min(64, uri.len())];
        eprintln!(
            "considering dependency uri (len={}) sample='{}'{}",
            uri.len(),
            sample,
            if uri.len() > 64 { "..." } else { "" }
        );

        let trimmed_uri = uri.trim();

        // Pre-decode checks: look for common markers (data:, percent-encoded
        // data markers, base64 tokens, comma markers) which indicate the
        // URI is not a filesystem dependency.
        if looks_like_embedded_payload(trimmed_uri) {
            eprintln!(
                "skipping embedded-like dependency (pre-decode) len={}",
                trimmed_uri.len()
            );
            continue;
        }

        if trimmed_uri.contains("://") || trimmed_uri.starts_with("//") {
            panic!("unsupported external dependency {uri}");
        }

        if trimmed_uri.len() > 1024 {
            eprintln!(
                "skipping suspiciously long dependency URI before decode (len={})",
                trimmed_uri.len()
            );
            continue;
        }

        let decoded = percent_decode_uri(&uri).expect("uri should decode");
        let decoded_trimmed = decoded.trim();

        // Post-decode checks: if the decoded string looks like an embedded
        // payload or is very long, skip it. Avoid printing the decoded
        // content to stdout to prevent huge logs.
        if looks_like_embedded_payload(decoded_trimmed) {
            eprintln!(
                "skipping embedded-like dependency (post-decode) decoded_len={}",
                decoded_trimmed.len()
            );
            continue;
        }

        if decoded_trimmed.len() > 4096 {
            eprintln!(
                "skipping dependency because decoded URI is suspiciously long (len={})",
                decoded_trimmed.len()
            );
            continue;
        }

        // If decoded is long, check how "base64-like" it is: if most
        // characters are base64 alphabet or padding, treat it as inline
        // payload and skip it.
        let is_base64_like = if decoded_trimmed.len() > 64 {
            let total = decoded_trimmed.chars().count();
            let good = decoded_trimmed
                .chars()
                .filter(|c| {
                    matches!(c, 'A'..='Z' | 'a'..='z' | '0'..='9' | '+' | '/' | '=' | '-' | '_')
                        || c.is_whitespace()
                })
                .count();
            // ratio threshold: 95%
            total > 0 && (good * 100) / total >= 95
        } else {
            false
        };

        if is_base64_like {
            eprintln!(
                "skipping dependency because decoded looks like inline base64/payload (len={})",
                decoded_trimmed.len()
            );
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

    let (document, buffers, _) =
        SceneLoader::import_gltf_native(&source_path).expect("import glTF for textures");

    // Debugging: print texture/image info when no embedded textures are found
    eprintln!("imported document has {} textures", document.textures().count());
    for (i, texture) in document.textures().enumerate() {
        let image = texture.source();
        match image.source() {
            gltf::image::Source::Uri { uri, .. } => {
                eprintln!("texture[{}] uri='{}'", i, uri);
            }
            gltf::image::Source::View { view, mime_type } => {
                eprintln!("texture[{}] view buffer_idx={} mime={:?}", i, view.buffer().index(), mime_type);
            }
        }
    }

    let textures_dir_rel = PathBuf::from("textures");
    let textures_dir_abs = asset_folder.join(&textures_dir_rel);
    let mut packaged_textures: Vec<PackagedTexture> = Vec::new();
    let mut texture_remap: HashMap<usize, (PathBuf, Option<String>)> = HashMap::new();
    let mut created_texture_dir = false;

    for texture in document.textures() {
        let image = texture.source();
        match image.source() {
            gltf::image::Source::Uri { uri, .. } => {
                let trimmed = uri.trim();
                if !trimmed.starts_with("data:") {
                    continue;
                }

                let (mime, data) = decode_embedded_data_uri(trimmed);

                if !created_texture_dir {
                    fs::create_dir_all(&textures_dir_abs).expect("create texture directory");
                    created_texture_dir = true;
                }

                let extension = extension_for_mime_type(mime.as_deref());
                let file_name = texture_file_name(texture.index(), &extension);
                let texture_rel = textures_dir_rel.join(&file_name);
                let texture_abs = asset_folder.join(&texture_rel);
                fs::write(&texture_abs, &data).expect("write embedded texture");

                let project_relative = project_relative_package.join(&texture_rel);
                let display_name = texture
                    .name()
                    .or_else(|| image.name())
                    .map(|name| name.to_string());

                texture_remap.insert(texture.index(), (project_relative, display_name.clone()));
                packaged_textures.push(PackagedTexture {
                    index: texture.index() as u32,
                    path: texture_rel,
                    name: display_name,
                });
            }
            gltf::image::Source::View { view, mime_type } => {
                let buffer_index = view.buffer().index();
                let buffer = &buffers[buffer_index];
                let start = view.offset();
                let end = start + view.length();
                assert!(end <= buffer.len(), "embedded texture view out of bounds");

                if !created_texture_dir {
                    fs::create_dir_all(&textures_dir_abs).expect("create texture directory");
                    created_texture_dir = true;
                }

                let extension = extension_for_mime_type(Some(mime_type));
                let file_name = texture_file_name(texture.index(), &extension);
                let texture_rel = textures_dir_rel.join(&file_name);
                let texture_abs = asset_folder.join(&texture_rel);
                fs::write(&texture_abs, &buffer[start..end]).expect("write embedded view texture");

                let project_relative = project_relative_package.join(&texture_rel);
                let display_name = texture
                    .name()
                    .or_else(|| image.name())
                    .map(|name| name.to_string());

                texture_remap.insert(texture.index(), (project_relative, display_name.clone()));
                packaged_textures.push(PackagedTexture {
                    index: texture.index() as u32,
                    path: texture_rel,
                    name: display_name,
                });
            }
        }
    }

    packaged_textures.sort_by_key(|entry| entry.index);

    if !texture_remap.is_empty() {
        let materials: Vec<_> = document.materials().collect();

        for entity in &mut stored_asset.entities {
            let Some(material) = entity.material_data.as_mut() else {
                continue;
            };

            let Some(material_index) = entity.gltf_material else {
                continue;
            };

            let Some(gltf_material) = materials.get(material_index) else {
                continue;
            };

            let pbr = gltf_material.pbr_metallic_roughness();

            if let Some(info) = pbr.base_color_texture() {
                if let Some((path, name)) = texture_remap.get(&info.texture().index()) {
                    let slot = &mut material.base_color_texture;
                    slot.path = Some(path.clone());
                    slot.name = name.clone();
                    slot.index = None;
                }
            }

            if let Some(info) = pbr.metallic_roughness_texture() {
                if let Some((path, name)) = texture_remap.get(&info.texture().index()) {
                    let slot = &mut material.metallic_roughness_texture;
                    slot.path = Some(path.clone());
                    slot.name = name.clone();
                    slot.index = None;
                }
            }

            if let Some(normal) = gltf_material.normal_texture() {
                if let Some((path, name)) = texture_remap.get(&normal.texture().index()) {
                    let slot = &mut material.normal_texture;
                    slot.path = Some(path.clone());
                    slot.name = name.clone();
                    slot.index = None;
                }
            }

            if let Some(emissive) = gltf_material.emissive_texture() {
                if let Some((path, name)) = texture_remap.get(&emissive.texture().index()) {
                    let slot = &mut material.emissive_texture;
                    slot.path = Some(path.clone());
                    slot.name = name.clone();
                    slot.index = None;
                }
            }

            if let Some(occlusion) = gltf_material.occlusion_texture() {
                if let Some((path, name)) = texture_remap.get(&occlusion.texture().index()) {
                    let slot = &mut material.occlusion_texture;
                    slot.path = Some(path.clone());
                    slot.name = name.clone();
                    slot.index = None;
                }
            }
        }
    }

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
            textures: packaged_textures,
        }),
    };

    let descriptor_json = serde_json::to_string_pretty(&descriptor).expect("serialize descriptor");
    fs::write(&descriptor_absolute, descriptor_json).expect("write descriptor");

    (project_relative_package, descriptor_relative, bundle)
}

#[test]
#[ignore]
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
        .join("web/assets/animated/AnimatedCube.gltf");
    let bin_source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("web/assets/animated/AnimatedCube.bin");
    let texture_source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("web/assets/animated/AnimatedCube_BaseColor.png");

    fs::copy(&gltf_source, source_root.join("AnimatedCube.gltf")).expect("copy gltf");
    fs::copy(&bin_source, source_root.join("AnimatedCube.bin")).expect("copy bin");
    fs::copy(
        &texture_source,
        source_root.join("AnimatedCube_BaseColor.png"),
    )
    .expect("copy texture");

    let gltf_copy_path = source_root.join("AnimatedCube.gltf");
    let texture_copy_path = source_root.join("AnimatedCube_BaseColor.png");
    let texture_bytes = fs::read(&texture_copy_path).expect("read embedded texture source");
    let data_uri = format!("data:image/png;base64,{}", encode(&texture_bytes));
    let mut gltf_json = fs::read_to_string(&gltf_copy_path).expect("read glTF source");
    gltf_json = gltf_json.replace("AnimatedCube_BaseColor.png", &data_uri);
    fs::write(&gltf_copy_path, gltf_json).expect("embed texture in glTF");
    fs::remove_file(&texture_copy_path).expect("remove texture source");

    set_active_project_root(Some(project_root.to_path_buf()));

    let gltf_path = source_root.join("AnimatedCube.gltf");
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
