use crate::environment::{ColorGrading, Environment, HdrBackground};
use crate::scene::loader::SceneImportDevice;
use crate::scene::{
    Scene, SceneAsset, SceneAssetBundle, SceneAssetResources, SerializedRuneScript,
    SerializedRuneScriptSource,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{OnceLock, RwLock};
use thiserror::Error;

pub const PROJECT_FILE_NAME: &str = "project.json";
pub const SCENE_FILE_NAME: &str = "scene.json";
pub const CONTENT_DIR: &str = "content";
const PROJECT_VERSION: u32 = 1;

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

pub(crate) fn normalize_absolute_path(path: PathBuf) -> PathBuf {
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
    #[error("failed to access the filesystem: {0}")]
    Io(#[from] std::io::Error),
    #[error("failed to (de)serialize project data: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("environment HDR image has no file name: {0}")]
    InvalidEnvironmentPath(PathBuf),
    #[error("environment HDR image not found: {0}")]
    MissingEnvironmentFile(PathBuf),
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectManifest {
    version: u32,
    pub metadata: ProjectMetadata,
    pub scene: SceneAsset,
    pub environment: SerializedEnvironment,
    #[serde(default)]
    pub imports: Vec<ProjectImport>,
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
        Self {
            version: PROJECT_VERSION,
            scene: SceneAsset::builder(metadata.name.clone()).build(),
            metadata,
            environment,
            imports: Vec::new(),
        }
    }

    pub fn capture(scene: &Scene, metadata: ProjectMetadata) -> Result<Self, ProjectError> {
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
        Ok(Self {
            version: PROJECT_VERSION,
            metadata,
            scene: asset,
            environment,
            imports,
        })
    }

    pub fn save_to_dir(&self, dir: &Path) -> Result<(), ProjectError> {
        fs::create_dir_all(dir)?;

        let mut manifest = self.clone();
        manifest.environment.prepare_for_save(dir)?;
        manifest.scene.persist_material_assets(dir)?;

        let json = serde_json::to_string_pretty(&manifest)?;
        fs::write(dir.join(PROJECT_FILE_NAME), json)?;

        let scene_json = manifest
            .scene
            .to_json()
            .map_err(ProjectError::Serialization)?;
        fs::write(dir.join(SCENE_FILE_NAME), scene_json)?;
        Ok(())
    }

    pub fn load_from_dir(dir: &Path) -> Result<Self, ProjectError> {
        let manifest_path = dir.join(PROJECT_FILE_NAME);
        let json = fs::read_to_string(&manifest_path)?;
        let mut manifest: ProjectManifest = serde_json::from_str(&json)?;
        manifest.environment.resolve_paths(dir)?;

        let scene_path = dir.join(SCENE_FILE_NAME);
        if scene_path.exists() {
            let scene_json = fs::read_to_string(scene_path)?;
            manifest.scene =
                SceneAsset::from_json(&scene_json).map_err(ProjectError::Serialization)?;
            manifest.scene.persist_material_assets(dir)?;
        }
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

    pub fn instantiate_into(
        &self,
        scene: &mut Scene,
        renderer: &mut impl SceneImportDevice,
        project_root: &Path,
    ) -> Result<bool, ProjectError> {
        set_active_project_root(Some(project_root.to_path_buf()));

        let mut new_scene = Scene::new();
        let environment = self.environment.clone().into_environment(project_root)?;
        new_scene.set_environment(environment);

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

        let mut bundle = SceneAssetBundle::new(self.scene.clone(), SceneAssetResources::default());

        let registration = bundle.register_resources(renderer, &mut new_scene.assets);
        let textures_changed = registration.textures_changed;

        if !bundle.asset.entities.is_empty() {
            let main = new_scene.instantiate_asset_with_renderer(&bundle.asset, None, renderer);
            new_scene.set_main_scene(main);
        }

        *scene = new_scene;
        Ok(textures_changed)
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
    use std::io::Write;

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
        let json = serde_json::to_string(&ProjectManifest {
            version: PROJECT_VERSION,
            metadata: ProjectMetadata::default(),
            scene: SceneAsset::builder("Test").build(),
            environment: serialized.clone(),
            imports: Vec::new(),
        })
        .unwrap();
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
}
