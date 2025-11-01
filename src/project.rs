use crate::environment::{ColorGrading, Environment, HdrBackground};
use crate::scene::loader::SceneImportDevice;
use crate::scene::{
    Camera, CameraProjection, Scene, SceneAsset, SceneLibrary, SceneWorkspace,
    SceneWorkspaceBuilder, SerializedRuneScript, SerializedRuneScriptSource,
};
use glam::Vec3;
use log::warn;
use rand::distributions::{Alphanumeric, DistString};
use rand::rngs::SmallRng;
use rand::SeedableRng;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{OnceLock, RwLock};
use thiserror::Error;

pub const PROJECT_FILE_NAME: &str = "project.json";
pub const SCENE_FILE_NAME: &str = "scene.json";
pub const SCENE_LIBRARY_DIR: &str = "scenes";
pub const CONTENT_DIR: &str = "content";
const PROJECT_VERSION: u32 = 1;

fn generate_scene_document_id() -> String {
    let mut rng = SmallRng::from_entropy();
    Alphanumeric.sample_string(&mut rng, 12)
}

fn default_scene_relative_path(document_id: &str) -> PathBuf {
    PathBuf::from(CONTENT_DIR)
        .join(SCENE_LIBRARY_DIR)
        .join(document_id)
        .join(SCENE_FILE_NAME)
}

fn collect_metadata_files(dir: &Path) -> Vec<PathBuf> {
    let mut stack = vec![dir.to_path_buf()];
    let mut files = Vec::new();

    while let Some(current) = stack.pop() {
        if let Ok(entries) = fs::read_dir(&current) {
            for entry in entries.flatten() {
                let path = entry.path();
                match entry.file_type() {
                    Ok(file_type) if file_type.is_dir() => stack.push(path),
                    Ok(file_type) if file_type.is_file() => {
                        if path.extension().and_then(|ext| ext.to_str()) == Some("json") {
                            files.push(path);
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    files
}

fn active_project_root_cell() -> &'static RwLock<Option<PathBuf>> {
    static ACTIVE_PROJECT_ROOT: OnceLock<RwLock<Option<PathBuf>>> = OnceLock::new();
    ACTIVE_PROJECT_ROOT.get_or_init(|| RwLock::new(None))
}

#[cfg(windows)]
const VERBATIM_PREFIX: &str = r"\\?\";
#[cfg(windows)]
const VERBATIM_UNC_PREFIX: &str = r"\\?\UNC\";

/// Sets the active project root used to resolve relative asset paths at runtime.
///
/// Passing `None` clears the project root, causing relative paths to resolve
/// against the current working directory instead.
pub fn set_active_project_root(root: Option<PathBuf>) {
    let cell = active_project_root_cell();
    let normalized = root.map(normalize_absolute_path);
    *cell.write().expect("project root lock poisoned") = normalized;
}

/// Returns the active project root if one has been configured.
pub fn active_project_root() -> Option<PathBuf> {
    let cell = active_project_root_cell();
    cell.read().expect("project root lock poisoned").clone()
}

/// Resolves a path relative to the active project root when available.
pub fn resolve_project_path(path: impl AsRef<Path>) -> PathBuf {
    let path = path.as_ref();
    if path.is_absolute() {
        return path.to_path_buf();
    }

    if let Some(root) = active_project_root() {
        return root.join(path);
    }

    path.to_path_buf()
}

/// Normalizes an absolute path for cross-platform comparisons.
///
/// On Windows this strips the verbatim `\\\\?\\` prefix returned by some
/// filesystem APIs so that string comparisons behave as expected. On other
/// platforms the input path is returned unchanged.
pub fn normalize_absolute_path(path: PathBuf) -> PathBuf {
    #[cfg(windows)]
    {
        use std::borrow::Cow;

        match path.as_os_str().to_string_lossy() {
            Cow::Owned(s) => strip_verbatim_prefix(&s),
            Cow::Borrowed(s) => strip_verbatim_prefix(s),
        }
    }

    #[cfg(not(windows))]
    {
        path
    }
}

pub(crate) fn relativize_path_to_project(
    path: PathBuf,
    project_root: &Path,
    canonical_project_root: Option<&Path>,
) -> PathBuf {
    if path.is_relative() {
        return path;
    }

    let normalized_path = normalize_absolute_path(path);
    let normalized_project_root = normalize_absolute_path(project_root.to_path_buf());

    let mut candidate_roots: Vec<PathBuf> = vec![normalized_project_root.clone()];

    if let Some(root) = canonical_project_root {
        candidate_roots.push(normalize_absolute_path(root.to_path_buf()));
    } else if let Ok(canonical) = std::fs::canonicalize(&normalized_project_root) {
        candidate_roots.push(normalize_absolute_path(canonical));
    }

    candidate_roots.dedup();

    for root in &candidate_roots {
        if let Ok(stripped) = normalized_path.strip_prefix(root) {
            return stripped.to_path_buf();
        }
    }

    let canonicalized_path = std::fs::canonicalize(&normalized_path)
        .map(normalize_absolute_path)
        .ok();

    if let Some(ref canonical_path) = canonicalized_path {
        for root in &candidate_roots {
            if let Ok(stripped) = canonical_path.strip_prefix(root) {
                return stripped.to_path_buf();
            }
        }
        return canonical_path.clone();
    }

    normalized_path
}

#[cfg(windows)]
fn strip_verbatim_prefix(path: &str) -> PathBuf {
    if let Some(stripped) = path.strip_prefix(VERBATIM_UNC_PREFIX) {
        let mut parts = stripped.splitn(3, '\\');
        let server = parts.next().unwrap_or_default();
        let share = parts.next().unwrap_or_default();
        let rest = parts.next().unwrap_or_default();

        let mut rebuilt = String::from(r"\\");
        rebuilt.push_str(server);
        if !share.is_empty() {
            rebuilt.push('\\');
            rebuilt.push_str(share);
        }
        if !rest.is_empty() {
            rebuilt.push('\\');
            rebuilt.push_str(rest);
        }
        PathBuf::from(rebuilt)
    } else if let Some(stripped) = path.strip_prefix(VERBATIM_PREFIX) {
        PathBuf::from(stripped)
    } else {
        PathBuf::from(path)
    }
}

#[derive(Debug, Error)]
pub enum ProjectError {
    #[error("scene has no serializable content")]
    EmptyScene,
    #[error("project manifest contains no scenes")]
    NoScenes,
    #[error("failed to access the filesystem: {0}")]
    Io(#[from] std::io::Error),
    #[error("failed to (de)serialize project data: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("environment HDR image has no file name: {0}")]
    InvalidEnvironmentPath(PathBuf),
    #[error("environment HDR image not found: {0}")]
    MissingEnvironmentFile(PathBuf),
    #[error("missing scene asset for document {0}")]
    MissingSceneAsset(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProjectMetadata {
    pub name: String,
    #[serde(default)]
    pub description: String,
}

impl Default for ProjectMetadata {
    fn default() -> Self {
        Self {
            name: "Untitled Project".to_string(),
            description: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct ProjectImport {
    pub package: PathBuf,
    #[serde(default)]
    pub metadata: Vec<PathBuf>,
}

mod relative_path_serde {
    use serde::{Deserialize, Deserializer, Serializer};
    use std::path::{Path, PathBuf};

    pub fn serialize<S>(path: &Path, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(path.to_string_lossy().as_ref())
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<PathBuf, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Ok(PathBuf::from(value))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct SceneDocumentDependencies {
    #[serde(default)]
    pub prefab_instances: BTreeSet<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SceneDocument {
    pub id: String,
    pub name: String,
    #[serde(with = "relative_path_serde")]
    pub relative_path: PathBuf,
    #[serde(default)]
    pub dependencies: SceneDocumentDependencies,
}

impl SceneDocument {
    fn with_asset(id: String, asset: &SceneAsset) -> Self {
        Self {
            name: asset.name.clone(),
            relative_path: default_scene_relative_path(&id),
            id,
            dependencies: SceneDocumentDependencies::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectManifest {
    version: u32,
    pub metadata: ProjectMetadata,
    #[serde(default)]
    pub scenes: Vec<SceneDocument>,
    pub environment: SerializedEnvironment,
    #[serde(default)]
    pub engine_camera: SerializedEngineCamera,
    #[serde(default)]
    pub imports: Vec<ProjectImport>,
    #[serde(skip, default)]
    scene_assets: HashMap<String, SceneAsset>,
}

impl ProjectManifest {
    /// Creates a manifest with default scene and environment data.
    ///
    /// # Examples
    ///
    /// ```
    /// use wgpu_cube::project::{ProjectManifest, ProjectMetadata};
    ///
    /// let manifest = ProjectManifest::new_empty(ProjectMetadata::default());
    /// assert_eq!(manifest.metadata.name, "Untitled Project");
    /// ```
    pub fn new_empty(metadata: ProjectMetadata) -> Self {
        let environment = SerializedEnvironment::from_environment(&Environment::default());
        let mut manifest = Self {
            version: PROJECT_VERSION,
            metadata,
            scenes: Vec::new(),
            environment,
            engine_camera: SerializedEngineCamera::default(),
            imports: Vec::new(),
            scene_assets: HashMap::new(),
        };

        let scene_asset = SceneAsset::builder(manifest.metadata.name.clone()).build();
        let document_id = generate_scene_document_id();
        let document = SceneDocument::with_asset(document_id.clone(), &scene_asset);
        manifest.scene_assets.insert(document_id, scene_asset);
        manifest.scenes.push(document);
        manifest
    }

    pub fn scenes(&self) -> &[SceneDocument] {
        &self.scenes
    }

    pub fn environment(&self) -> &SerializedEnvironment {
        &self.environment
    }

    pub fn engine_camera(&self) -> &SerializedEngineCamera {
        &self.engine_camera
    }

    pub fn scene_asset(&self, document_id: &str) -> Option<&SceneAsset> {
        self.scene_assets.get(document_id)
    }

    pub fn capture(
        scene: &Scene,
        metadata: ProjectMetadata,
        existing_document: Option<&SceneDocument>,
    ) -> Result<Self, ProjectError> {
        let engine_camera = SerializedEngineCamera::from_camera(scene.camera());
        let mut asset = scene
            .export_main_asset(metadata.name.clone())
            .ok_or(ProjectError::EmptyScene)?;
        let environment = SerializedEnvironment::from_environment(scene.environment());
        let mut package_dirs: BTreeSet<PathBuf> = BTreeSet::new();
        let mut referenced_mesh_indices: HashSet<usize> = HashSet::new();
        let project_root = active_project_root();
        let canonical_project_root = project_root
            .as_ref()
            .and_then(|root| std::fs::canonicalize(root).ok());

        let script_references_gltf = |script: &SerializedRuneScript| -> bool {
            match &script.source {
                SerializedRuneScriptSource::Inline { source, .. } => source.contains("import_gltf"),
                SerializedRuneScriptSource::File { path } => path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .map(|name| name == "editor_import_gltf.rn")
                    .unwrap_or(false),
            }
        };

        for entity in &mut asset.entities {
            if let Some(source) = entity.gltf_source.clone() {
                let mut manifest_path = source.clone();

                if source.is_absolute() {
                    if let Some(root) = project_root.as_ref() {
                        manifest_path = relativize_path_to_project(
                            source.clone(),
                            root,
                            canonical_project_root.as_deref(),
                        );
                    }
                }

                if let Some(parent) = manifest_path.parent() {
                    if !parent.as_os_str().is_empty() {
                        package_dirs.insert(parent.to_path_buf());
                    }
                }

                entity.gltf_source = Some(manifest_path);
            }

            if entity.primitive_mesh.is_some() {
                entity.mesh_handle = None;
            } else if let Some(mesh_handle) = entity.mesh_handle {
                referenced_mesh_indices.insert(mesh_handle);
            }

            if let Some(script) = entity.script.as_ref() {
                if script_references_gltf(script) {
                    entity.script = None;
                }
            }
        }

        if !asset.mesh_data.is_empty() {
            let original_mesh_data = std::mem::take(&mut asset.mesh_data);
            let mut remap: HashMap<usize, usize> = HashMap::new();
            let mut retained_meshes = Vec::with_capacity(referenced_mesh_indices.len());

            for (index, data) in original_mesh_data.into_iter().enumerate() {
                if referenced_mesh_indices.contains(&index) {
                    let new_index = retained_meshes.len();
                    remap.insert(index, new_index);
                    retained_meshes.push(data);
                }
            }

            asset.mesh_data = retained_meshes;

            for entity in &mut asset.entities {
                if entity.primitive_mesh.is_none() {
                    if let Some(mesh_handle) = entity.mesh_handle {
                        if let Some(&new_index) = remap.get(&mesh_handle) {
                            entity.mesh_handle = Some(new_index);
                        } else if entity.gltf_source.is_some() {
                            entity.mesh_handle = None;
                        }
                    }
                } else {
                    entity.mesh_handle = None;
                }
            }
        }

        let mut imports: Vec<ProjectImport> = Vec::new();

        for package in package_dirs.into_iter() {
            let mut metadata_paths: Vec<PathBuf> = Vec::new();

            if let Some(root) = project_root.as_ref() {
                let absolute_package = root.join(&package);
                for metadata in collect_metadata_files(&absolute_package) {
                    if let Ok(relative) = metadata.strip_prefix(root) {
                        metadata_paths.push(relative.to_path_buf());
                    }
                }
            }

            metadata_paths.sort();

            imports.push(ProjectImport {
                package,
                metadata: metadata_paths,
            });
        }
        let (document, document_id) = if let Some(existing) = existing_document {
            let mut document = existing.clone();
            document.name = asset.name.clone();
            (document, existing.id.clone())
        } else {
            let document_id = generate_scene_document_id();
            let document = SceneDocument::with_asset(document_id.clone(), &asset);
            (document, document_id)
        };

        let mut scene_assets = HashMap::new();
        scene_assets.insert(document_id.clone(), asset);

        Ok(Self {
            version: PROJECT_VERSION,
            metadata,
            scenes: vec![document],
            environment,
            engine_camera,
            imports,
            scene_assets,
        })
    }

    pub fn save_to_dir(&self, dir: &Path) -> Result<(), ProjectError> {
        fs::create_dir_all(dir)?;

        let mut manifest = self.clone();
        manifest.environment.prepare_for_save(dir)?;
        for asset in manifest.scene_assets.values_mut() {
            asset.persist_material_assets(dir)?;
        }

        let json = serde_json::to_string_pretty(&manifest)?;
        fs::write(dir.join(PROJECT_FILE_NAME), json)?;

        for document in &manifest.scenes {
            let Some(asset) = manifest.scene_assets.get(&document.id) else {
                return Err(ProjectError::MissingSceneAsset(document.id.clone()));
            };
            let scene_json = asset.to_json().map_err(ProjectError::Serialization)?;
            let scene_path = dir.join(&document.relative_path);
            if let Some(parent) = scene_path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(scene_path, scene_json)?;
        }
        let legacy_scene = dir.join(SCENE_FILE_NAME);
        if legacy_scene.exists() {
            let _ = fs::remove_file(legacy_scene);
        }
        Ok(())
    }

    pub fn load_from_dir(dir: &Path) -> Result<Self, ProjectError> {
        let manifest_path = dir.join(PROJECT_FILE_NAME);
        let json = fs::read_to_string(&manifest_path)?;
        let mut manifest: ProjectManifest = serde_json::from_str(&json)?;
        manifest.environment.resolve_paths(dir)?;
        manifest.scene_assets.clear();

        let mut documents: Vec<SceneDocument> = Vec::new();
        let mut existing: BTreeMap<String, SceneDocument> = manifest
            .scenes
            .iter()
            .cloned()
            .map(|doc| (doc.id.clone(), doc))
            .collect();

        let legacy_scene_path = dir.join(SCENE_FILE_NAME);
        if legacy_scene_path.exists() {
            let scene_json = fs::read_to_string(&legacy_scene_path)?;
            let mut asset =
                SceneAsset::from_json(&scene_json).map_err(ProjectError::Serialization)?;
            asset.persist_material_assets(dir)?;
            let document_id = existing
                .keys()
                .next()
                .cloned()
                .unwrap_or_else(generate_scene_document_id);
            let (dependencies, relative_path) = match existing.remove(&document_id) {
                Some(doc) => (doc.dependencies, doc.relative_path),
                None => (
                    SceneDocumentDependencies::default(),
                    default_scene_relative_path(&document_id),
                ),
            };
            let absolute_path = dir.join(&relative_path);
            if let Some(parent) = absolute_path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&absolute_path, scene_json)?;
            let _ = fs::remove_file(&legacy_scene_path);
            let document_name = asset.name.clone();
            manifest.scene_assets.insert(document_id.clone(), asset);
            documents.push(SceneDocument {
                id: document_id.clone(),
                name: document_name,
                relative_path,
                dependencies,
            });
        }

        let scenes_root = dir.join(CONTENT_DIR).join(SCENE_LIBRARY_DIR);
        if scenes_root.exists() {
            for entry in fs::read_dir(&scenes_root)? {
                let entry = entry?;
                if !entry.file_type()?.is_dir() {
                    continue;
                }

                let document_id = match entry.file_name().into_string() {
                    Ok(id) => id,
                    Err(name) => {
                        warn!(
                            "Skipping scene directory with invalid UTF-8 name: {:?}",
                            name
                        );
                        continue;
                    }
                };

                let relative_path = default_scene_relative_path(&document_id);
                let scene_path = dir.join(&relative_path);
                if !scene_path.exists() {
                    warn!(
                        "Missing scene document for id {} at {:?}",
                        document_id, scene_path
                    );
                    continue;
                }

                let scene_json = fs::read_to_string(&scene_path)?;
                let mut asset =
                    SceneAsset::from_json(&scene_json).map_err(ProjectError::Serialization)?;
                asset.persist_material_assets(dir)?;
                let document_name = asset.name.clone();
                manifest.scene_assets.insert(document_id.clone(), asset);
                let (dependencies, relative_path) = match existing.remove(&document_id) {
                    Some(doc) => (doc.dependencies, doc.relative_path),
                    None => (
                        SceneDocumentDependencies::default(),
                        default_scene_relative_path(&document_id),
                    ),
                };
                documents.push(SceneDocument {
                    id: document_id,
                    name: document_name,
                    relative_path,
                    dependencies,
                });
            }
        }

        documents.sort_by(|a, b| a.id.cmp(&b.id));
        manifest.scenes = documents;
        Ok(manifest)
    }

    pub fn from_json_str(json: &str) -> Result<Self, ProjectError> {
        let manifest: ProjectManifest = serde_json::from_str(json)?;
        Ok(manifest)
    }

    pub fn from_json_bytes(bytes: &[u8]) -> Result<Self, ProjectError> {
        let manifest: ProjectManifest = serde_json::from_slice(bytes)?;
        Ok(manifest)
    }

    pub fn instantiate_workspace(
        &self,
        renderer: &mut impl SceneImportDevice,
        project_root: &Path,
        library: &mut SceneLibrary,
    ) -> Result<(SceneWorkspace, bool), ProjectError> {
        set_active_project_root(Some(project_root.to_path_buf()));

        for import in &self.imports {
            for meta in &import.metadata {
                let path = if meta.is_absolute() {
                    meta.clone()
                } else {
                    project_root.join(meta)
                };

                if !path.exists() {
                    log::warn!(
                        "Missing metadata file referenced by project import: {:?}",
                        path
                    );
                }
            }
        }

        let builder = SceneWorkspaceBuilder::new(self, project_root, library);
        builder.build(renderer)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializedEngineCamera {
    eye: [f32; 3],
    target: [f32; 3],
    up: [f32; 3],
    projection: CameraProjection,
}

impl SerializedEngineCamera {
    pub fn from_camera(camera: &Camera) -> Self {
        Self {
            eye: camera.eye.to_array(),
            target: camera.target.to_array(),
            up: camera.up.to_array(),
            projection: camera.projection(),
        }
    }

    pub fn to_camera(&self) -> Camera {
        let mut camera = Camera::default();
        camera.eye = Vec3::from_array(self.eye);
        camera.target = Vec3::from_array(self.target);
        camera.up = Vec3::from_array(self.up);
        camera.set_projection(self.projection);
        camera
    }
}

impl Default for SerializedEngineCamera {
    fn default() -> Self {
        Self::from_camera(&Camera::default())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializedEnvironment {
    clear_color: [f64; 4],
    ambient_intensity: f32,
    color_grading: SerializedColorGrading,
    #[serde(default)]
    hdr: Option<SerializedHdrBackground>,
}

impl SerializedEnvironment {
    fn from_environment(environment: &Environment) -> Self {
        let clear = environment.clear_color();
        let grading = environment.color_grading();
        let hdr = environment
            .hdr_background()
            .map(|background| SerializedHdrBackground {
                enabled: background.enabled(),
                image_path: Some(background.path().to_path_buf()),
                intensity: background.intensity(),
                source_path: Some(background.path().to_path_buf()),
            });

        Self {
            clear_color: [clear.r, clear.g, clear.b, clear.a],
            ambient_intensity: environment.ambient_intensity(),
            color_grading: SerializedColorGrading::from(grading),
            hdr,
        }
    }

    pub fn to_environment(&self, project_root: &Path) -> Result<Environment, ProjectError> {
        self.clone().into_environment(project_root)
    }

    fn prepare_for_save(&mut self, project_dir: &Path) -> Result<(), ProjectError> {
        if let Some(hdr) = self.hdr.as_mut() {
            let Some(mut source) = hdr.source_path.clone().or_else(|| hdr.image_path.clone())
            else {
                return Ok(());
            };

            if !source.is_absolute() {
                source = std::env::current_dir()?.join(source);
            }

            if !source.exists() {
                return Err(ProjectError::MissingEnvironmentFile(source));
            }

            let Some(file_name) = source.file_name() else {
                return Err(ProjectError::InvalidEnvironmentPath(source));
            };

            let content_dir = project_dir.join(CONTENT_DIR);
            let environment_dir = content_dir.join("environment");
            fs::create_dir_all(&environment_dir)?;
            let target = environment_dir.join(file_name);
            let requires_copy = match paths_refer_to_same_file(&source, &target) {
                Ok(true) => false,
                Ok(false) => true,
                Err(err) => return Err(ProjectError::Io(err)),
            };

            if requires_copy {
                fs::copy(&source, &target)?;
            }

            hdr.image_path = Some(
                PathBuf::from(CONTENT_DIR)
                    .join("environment")
                    .join(file_name),
            );
        }

        Ok(())
    }

    fn resolve_paths(&mut self, project_dir: &Path) -> Result<(), ProjectError> {
        if let Some(hdr) = self.hdr.as_mut() {
            if let Some(path) = hdr.image_path.clone() {
                let absolute = if path.is_absolute() {
                    path
                } else {
                    project_dir.join(path)
                };

                #[cfg(not(target_arch = "wasm32"))]
                if !absolute.exists() {
                    return Err(ProjectError::MissingEnvironmentFile(absolute));
                }

                hdr.source_path = Some(absolute);
            }
        }

        Ok(())
    }

    fn into_environment(self, project_root: &Path) -> Result<Environment, ProjectError> {
        let mut environment = Environment::new(wgpu::Color {
            r: self.clear_color[0],
            g: self.clear_color[1],
            b: self.clear_color[2],
            a: self.clear_color[3],
        });
        environment.set_ambient_intensity(self.ambient_intensity);
        environment.set_color_grading(self.color_grading.into());

        if let Some(hdr) = self.hdr {
            if let Some(path) = hdr
                .source_path
                .or_else(|| hdr.image_path.map(|p| project_root.join(p)))
            {
                if !path.exists() {
                    return Err(ProjectError::MissingEnvironmentFile(path));
                }

                let mut background = HdrBackground::new(path);
                background.set_enabled(hdr.enabled);
                background.set_intensity(hdr.intensity);
                environment.set_hdr_background(Some(background));
            }
        }

        Ok(environment)
    }
}

fn paths_refer_to_same_file(source: &Path, target: &Path) -> std::io::Result<bool> {
    // Check if target exists first
    match fs::metadata(target) {
        Ok(_) => {}
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(err) => return Err(err),
    };

    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;

        let source_meta = fs::metadata(source)?;
        let target_meta = fs::metadata(target)?;

        Ok(source_meta.ino() == target_meta.ino() && source_meta.dev() == target_meta.dev())
    }

    #[cfg(not(unix))]
    {
        // For Windows and other platforms, use canonicalization
        // This is the most reliable cross-platform approach using only stable APIs
        let source_path = std::fs::canonicalize(source)?;
        let target_path = std::fs::canonicalize(target)?;
        Ok(source_path == target_path)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SerializedColorGrading {
    exposure: f32,
    saturation: f32,
    contrast: f32,
}

impl From<ColorGrading> for SerializedColorGrading {
    fn from(grading: ColorGrading) -> Self {
        Self {
            exposure: grading.exposure(),
            saturation: grading.saturation(),
            contrast: grading.contrast(),
        }
    }
}

impl From<SerializedColorGrading> for ColorGrading {
    fn from(serialized: SerializedColorGrading) -> Self {
        let mut grading = ColorGrading::default();
        grading.set_exposure(serialized.exposure);
        grading.set_saturation(serialized.saturation);
        grading.set_contrast(serialized.contrast);
        grading
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SerializedHdrBackground {
    enabled: bool,
    image_path: Option<PathBuf>,
    intensity: f32,
    #[serde(skip)]
    source_path: Option<PathBuf>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use pollster::block_on;
    use std::io::Write;
    use std::sync::Arc;
    use wgpu::{
        Backends, DeviceDescriptor, Features, Instance, InstanceDescriptor, Limits, MemoryHints,
        PowerPreference, RequestAdapterOptions, Trace,
    };

    use crate::asset::Mesh;
    use crate::renderer::Vertex;
    use crate::scene::{SceneAsset, SceneImportDevice, SceneLibrary};

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
                    eprintln!("Skipping project workspace test: failed to request adapter ({err})");
                    return None;
                }
            };

            let (device, queue) = block_on(adapter.request_device(&DeviceDescriptor {
                label: Some("project-workspace-test"),
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

    #[test]
    fn environment_roundtrip_preserves_hdr_path() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let hdr_path = tmp_dir.path().join("env.hdr");
        let mut file = fs::File::create(&hdr_path).unwrap();
        writeln!(file, "dummy").unwrap();

        let mut environment = Environment::default();
        environment.enable_hdr_background(&hdr_path);
        environment.hdr_background_mut().unwrap().set_intensity(2.0);

        let mut serialized = SerializedEnvironment::from_environment(&environment);
        serialized.prepare_for_save(tmp_dir.path()).unwrap();
        let stored_path = serialized
            .hdr
            .as_ref()
            .and_then(|hdr| hdr.image_path.clone())
            .expect("serialized environment should contain HDR path");
        assert_eq!(
            stored_path,
            PathBuf::from(CONTENT_DIR)
                .join("environment")
                .join("env.hdr")
        );

        let manifest_path = tmp_dir.path().join(PROJECT_FILE_NAME);
        let mut manifest = ProjectManifest::new_empty(ProjectMetadata::default());
        manifest.environment = serialized.clone();
        let json = serde_json::to_string(&manifest).unwrap();
        fs::write(&manifest_path, json).unwrap();

        let mut loaded_env = serialized;
        loaded_env.resolve_paths(tmp_dir.path()).unwrap();
        let environment = loaded_env.into_environment(tmp_dir.path()).unwrap();
        let background = environment.hdr_background().unwrap();
        assert!(background.path().ends_with("env.hdr"));
    }

    #[test]
    fn same_file_detection_handles_equivalent_paths() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let file_path = tmp_dir.path().join("env.hdr");
        fs::write(&file_path, b"dummy").unwrap();

        let via_parent = tmp_dir
            .path()
            .parent()
            .unwrap()
            .join(tmp_dir.path().file_name().unwrap())
            .join("env.hdr");

        assert!(super::paths_refer_to_same_file(&file_path, &via_parent).unwrap());

        let missing = tmp_dir.path().join("missing.hdr");
        assert!(!super::paths_refer_to_same_file(&file_path, &missing).unwrap());

        let other = tmp_dir.path().join("other.hdr");
        fs::write(&other, b"dummy").unwrap();
        assert!(!super::paths_refer_to_same_file(&file_path, &other).unwrap());
    }

    #[test]
    fn instantiate_workspace_registers_all_documents() {
        let Some(mut device) = HeadlessDevice::new() else {
            return;
        };

        let mut manifest = ProjectManifest::new_empty(ProjectMetadata::default());
        let secondary_asset = SceneAsset::builder("Secondary".to_string()).build();
        let secondary_id = generate_scene_document_id();
        let secondary_document = SceneDocument::with_asset(secondary_id.clone(), &secondary_asset);

        manifest
            .scene_assets
            .insert(secondary_id.clone(), secondary_asset);
        manifest.scenes.push(secondary_document);

        let mut library = SceneLibrary::new();
        let (workspace, _) = manifest
            .instantiate_workspace(&mut device, Path::new("."), &mut library)
            .expect("workspace instantiation should succeed");

        let handles: Vec<_> = workspace.scene_handles().collect();
        assert_eq!(handles.len(), 2, "workspace should contain both scenes");
        assert!(
            library.asset(&manifest.scenes[0].id).is_some(),
            "primary scene asset should be registered"
        );
        assert!(
            library.asset(&secondary_id).is_some(),
            "secondary scene asset should be registered"
        );
    }
}
