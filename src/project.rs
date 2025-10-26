use crate::environment::{ColorGrading, Environment, HdrBackground};
use crate::renderer::Renderer;
use crate::scene::{
    Scene, SceneAsset, SceneAssetBundle, SceneAssetResources, SceneLoader, SerializedMaterial,
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

fn active_project_root_cell() -> &'static RwLock<Option<PathBuf>> {
    static ACTIVE_PROJECT_ROOT: OnceLock<RwLock<Option<PathBuf>>> = OnceLock::new();
    ACTIVE_PROJECT_ROOT.get_or_init(|| RwLock::new(None))
}

/// Sets the active project root used to resolve relative asset paths at runtime.
///
/// Passing `None` clears the project root, causing relative paths to resolve
/// against the current working directory instead.
pub fn set_active_project_root(root: Option<PathBuf>) {
    let cell = active_project_root_cell();
    *cell.write().expect("project root lock poisoned") = root;
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
    pub path: PathBuf,
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
        let mut imports_set: BTreeSet<PathBuf> = BTreeSet::new();
        let mut referenced_mesh_indices: HashSet<usize> = HashSet::new();

        for entity in &mut asset.entities {
            if let Some(source) = &entity.gltf_source {
                imports_set.insert(source.clone());
            }

            if entity.primitive_mesh.is_some() {
                entity.mesh_handle = None;
            } else if let Some(mesh_handle) = entity.mesh_handle {
                referenced_mesh_indices.insert(mesh_handle);
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

        let imports = imports_set
            .into_iter()
            .map(|path| ProjectImport { path })
            .collect();
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
        renderer: &mut Renderer,
        project_root: &Path,
    ) -> Result<bool, ProjectError> {
        set_active_project_root(Some(project_root.to_path_buf()));

        let mut new_scene = Scene::new();
        let environment = self.environment.clone().into_environment(project_root)?;
        new_scene.set_environment(environment);

        let mut textures_changed = false;

        let mut mesh_lookup: HashMap<(PathBuf, Option<usize>, Option<usize>), usize> =
            HashMap::new();
        let mut material_lookup: HashMap<(PathBuf, usize), SerializedMaterial> = HashMap::new();

        for import in &self.imports {
            match SceneLoader::load_gltf_asset(&import.path, renderer, 1.0) {
                Ok(mut bundle) => {
                    let registration = bundle.register_resources(renderer, &mut new_scene.assets);
                    textures_changed |= registration.textures_changed;

                    for entity in &bundle.asset.entities {
                        let Some(source) = &entity.gltf_source else {
                            continue;
                        };

                        if let Some(mesh_handle) = entity.mesh_handle {
                            mesh_lookup.insert(
                                (source.clone(), entity.gltf_node, entity.gltf_primitive),
                                mesh_handle,
                            );
                        }

                        if let Some(gltf_material) = entity.gltf_material {
                            if let Some(material) = &entity.material {
                                material_lookup
                                    .insert((source.clone(), gltf_material), material.clone());
                            }
                        }
                    }
                }
                Err(err) => {
                    log::error!(
                        "Failed to reimport glTF {:?} while instantiating project: {}",
                        import.path,
                        err
                    );
                }
            }
        }

        let mut bundle = SceneAssetBundle::new(self.scene.clone(), SceneAssetResources::default());

        for entity in &mut bundle.asset.entities {
            let Some(source) = entity.gltf_source.clone() else {
                continue;
            };

            let mesh_key = (source.clone(), entity.gltf_node, entity.gltf_primitive);
            if let Some(&mesh_handle) = mesh_lookup.get(&mesh_key) {
                entity.mesh_handle = Some(mesh_handle);
            } else if entity.mesh_handle.is_some() {
                log::warn!(
                    "Missing reimported mesh for {:?} (node {:?}, primitive {:?})",
                    source,
                    entity.gltf_node,
                    entity.gltf_primitive
                );
            }

            if let Some(gltf_material) = entity.gltf_material {
                if let Some(material) = material_lookup.get(&(source.clone(), gltf_material)) {
                    entity.material = Some(material.clone());
                } else if entity.material.is_some() {
                    log::warn!(
                        "Missing reimported material for {:?} (material index {})",
                        source,
                        gltf_material
                    );
                }
            }
        }

        let registration = bundle.register_resources(renderer, &mut new_scene.assets);
        textures_changed |= registration.textures_changed;

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
            if source != target {
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
}
